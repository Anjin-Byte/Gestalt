use bytemuck;
use wgpu::util::DeviceExt;

use crate::core::{DispatchStats, MeshInput, TileSpec, VoxelGridSpec, VoxelizationOutput, VoxelizeOpts};
use crate::csr::build_tile_csr;

use super::buffers::map_buffer_u32;
use super::{GpuVoxelizer, Params};

impl GpuVoxelizer {
    pub async fn voxelize_surface(
        &self,
        mesh: &MeshInput,
        grid: &VoxelGridSpec,
        tiles: &TileSpec,
        opts: &VoxelizeOpts,
    ) -> Result<VoxelizationOutput, String> {
        grid.validate()?;
        mesh.validate()?;
        tiles.validate(self.max_invocations)?;

        let csr = build_tile_csr(mesh, grid, tiles, opts.epsilon);
        let num_tiles = tiles.num_tiles_total();

        let to_grid = grid.world_to_grid_matrix();
        let mut tri_data = Vec::with_capacity(mesh.triangles.len() * 6);
        for tri in &mesh.triangles {
            let p0 = to_grid.transform_point3(tri[0]);
            let p1 = to_grid.transform_point3(tri[1]);
            let p2 = to_grid.transform_point3(tri[2]);
            let min = p0.min(p1).min(p2);
            let max = p0.max(p1).max(p2);
            let normal = (p1 - p0).cross(p2 - p0);
            let d = -normal.dot(p0);
            tri_data.push([p0.x, p0.y, p0.z, 0.0]);
            tri_data.push([p1.x, p1.y, p1.z, 0.0]);
            tri_data.push([p2.x, p2.y, p2.z, 0.0]);
            tri_data.push([min.x, min.y, min.z, 0.0]);
            tri_data.push([max.x, max.y, max.z, 0.0]);
            tri_data.push([normal.x, normal.y, normal.z, d]);
        }

        let num_voxels = grid.num_voxels() as usize;
        let word_count = (num_voxels + 31) / 32;
        let occupancy_bytes = (word_count as u64).saturating_mul(4);
        self.ensure_storage_fits(occupancy_bytes, "dense occupancy")?;
        if opts.store_owner {
            let owner_bytes = (num_voxels as u64).saturating_mul(4);
            self.ensure_storage_fits(owner_bytes, "dense owner")?;
        }
        if opts.store_color {
            let color_bytes = (num_voxels as u64).saturating_mul(4);
            self.ensure_storage_fits(color_bytes, "dense color")?;
        }

        let tri_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.triangles"),
            contents: bytemuck::cast_slice(&tri_data),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let offsets_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.tile_offsets"),
            contents: bytemuck::cast_slice(&csr.tile_offsets),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let tri_indices_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.tri_indices"),
            contents: bytemuck::cast_slice(&csr.tri_indices),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let occupancy_init = vec![0u32; word_count];
        let occupancy_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.occupancy"),
            contents: bytemuck::cast_slice(&occupancy_init),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });

        let owner_init = vec![u32::MAX; num_voxels];
        let owner_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.owner"),
            contents: bytemuck::cast_slice(&owner_init),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });

        let color_init = vec![0u32; num_voxels];
        let color_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.color"),
            contents: bytemuck::cast_slice(&color_init),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });

        let params = Params {
            grid_dims: [grid.dims[0], grid.dims[1], grid.dims[2], 0],
            tile_dims: [tiles.tile_dims[0], tiles.tile_dims[1], tiles.tile_dims[2], 0],
            num_tiles_xyz: [tiles.num_tiles[0], tiles.num_tiles[1], tiles.num_tiles[2], 0],
            num_triangles: mesh.triangles.len() as u32,
            num_tiles,
            tile_voxels: tiles.tile_dims[0] * tiles.tile_dims[1] * tiles.tile_dims[2],
            store_owner: if opts.store_owner { 1 } else { 0 },
            store_color: if opts.store_color { 1 } else { 0 },
            debug: 0,
            _pad0: [0, 0],
        };
        let params_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let dummy_brick_origin = [[0u32, 0u32, 0u32, 0u32]];
        let brick_origins_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.brick_origins_dummy"),
            contents: bytemuck::cast_slice(&dummy_brick_origin),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let debug_init = [0u32, 0u32, 0u32];
        let debug_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.debug"),
            contents: bytemuck::cast_slice(&debug_init),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("voxelizer.bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: tri_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: offsets_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: tri_indices_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: occupancy_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: owner_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: color_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: params_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 10, resource: brick_origins_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 11, resource: debug_buf.as_entire_binding() },
            ],
        });

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("voxelizer.encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("voxelizer.pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            let workgroups = (num_tiles + self.tiles_per_workgroup - 1) / self.tiles_per_workgroup;
            self.ensure_workgroups_fit(workgroups, "dense dispatch")?;
            pass.dispatch_workgroups(workgroups, 1, 1);
        }

        let read_occupancy = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.read_occupancy"),
            size: (word_count * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let read_owner = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.read_owner"),
            size: (num_voxels * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let read_color = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.read_color"),
            size: (num_voxels * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let read_debug = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.read_debug"),
            size: 12,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        encoder.copy_buffer_to_buffer(&occupancy_buf, 0, &read_occupancy, 0, read_occupancy.size());
        encoder.copy_buffer_to_buffer(&owner_buf, 0, &read_owner, 0, read_owner.size());
        encoder.copy_buffer_to_buffer(&color_buf, 0, &read_color, 0, read_color.size());
        encoder.copy_buffer_to_buffer(&debug_buf, 0, &read_debug, 0, read_debug.size());

        self.queue.submit([encoder.finish()]);
        self.device.poll(wgpu::Maintain::Wait);

        let occupancy = map_buffer_u32(&read_occupancy, &self.device).await;
        let owner = map_buffer_u32(&read_owner, &self.device).await;
        let color = map_buffer_u32(&read_color, &self.device).await;
        let _debug = map_buffer_u32(&read_debug, &self.device).await;

        Ok(VoxelizationOutput {
            occupancy,
            owner_id: if opts.store_owner { Some(owner) } else { None },
            color_rgba: if opts.store_color { Some(color) } else { None },
            stats: DispatchStats {
                triangles: mesh.triangles.len() as u32,
                tiles: num_tiles,
                voxels: num_voxels as u32,
                gpu_time_ms: None,
            },
        })
    }
}
