//! Dense voxelization - full grid output.

use bytemuck;
use wgpu::util::DeviceExt;

use crate::core::{
    DispatchStats, MeshInput, TileSpec, VoxelGridSpec, VoxelizationOutput, VoxelizeOpts,
};
use crate::csr::build_tile_csr;

use super::map_buffer_u32;
use super::{GpuVoxelizer, Params};

impl GpuVoxelizer {
    /// Voxelizes a mesh surface into a dense voxel grid.
    ///
    /// Returns occupancy bitfield, optional owner IDs, and optional colors.
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

        self.validate_dense_storage(grid, opts)?;

        let csr = build_tile_csr(mesh, grid, tiles, opts.epsilon);
        let tri_data = prepare_triangle_data(mesh, grid);
        let buffers = self.create_dense_buffers(mesh, grid, tiles, opts, &csr, &tri_data);
        let bind_group = self.create_dense_bind_group(&buffers);

        self.dispatch_dense(&bind_group, tiles)?;

        let output = self.readback_dense(&buffers, grid, mesh, tiles, opts).await;
        Ok(output)
    }
}

// === Validation ===

impl GpuVoxelizer {
    fn validate_dense_storage(
        &self,
        grid: &VoxelGridSpec,
        opts: &VoxelizeOpts,
    ) -> Result<(), String> {
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
        Ok(())
    }
}

// === Triangle Data Preparation ===

fn prepare_triangle_data(mesh: &MeshInput, grid: &VoxelGridSpec) -> Vec<[f32; 4]> {
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

    tri_data
}

// === Buffer Creation ===

struct DenseBuffers {
    triangles: wgpu::Buffer,
    tile_offsets: wgpu::Buffer,
    tri_indices: wgpu::Buffer,
    occupancy: wgpu::Buffer,
    owner: wgpu::Buffer,
    color: wgpu::Buffer,
    params: wgpu::Buffer,
    brick_origins: wgpu::Buffer,
    debug: wgpu::Buffer,
    word_count: usize,
    num_voxels: usize,
}

impl GpuVoxelizer {
    fn create_dense_buffers(
        &self,
        mesh: &MeshInput,
        grid: &VoxelGridSpec,
        tiles: &TileSpec,
        opts: &VoxelizeOpts,
        csr: &crate::csr::TileTriangleCsr,
        tri_data: &[[f32; 4]],
    ) -> DenseBuffers {
        let num_voxels = grid.num_voxels() as usize;
        let word_count = (num_voxels + 31) / 32;

        let triangles = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxelizer.triangles"),
                contents: bytemuck::cast_slice(tri_data),
                usage: wgpu::BufferUsages::STORAGE,
            });

        let tile_offsets = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxelizer.tile_offsets"),
                contents: bytemuck::cast_slice(&csr.tile_offsets),
                usage: wgpu::BufferUsages::STORAGE,
            });

        let tri_indices = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxelizer.tri_indices"),
                contents: bytemuck::cast_slice(&csr.tri_indices),
                usage: wgpu::BufferUsages::STORAGE,
            });

        let occupancy_init = vec![0u32; word_count];
        let occupancy = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxelizer.occupancy"),
                contents: bytemuck::cast_slice(&occupancy_init),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });

        let owner_init = vec![u32::MAX; num_voxels];
        let owner = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxelizer.owner"),
                contents: bytemuck::cast_slice(&owner_init),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });

        let color_init = vec![0u32; num_voxels];
        let color = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxelizer.color"),
                contents: bytemuck::cast_slice(&color_init),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });

        let params = self.create_dense_params(mesh, grid, tiles, opts);
        let params_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxelizer.params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let dummy_brick_origin = [[0u32, 0u32, 0u32, 0u32]];
        let brick_origins = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxelizer.brick_origins_dummy"),
                contents: bytemuck::cast_slice(&dummy_brick_origin),
                usage: wgpu::BufferUsages::STORAGE,
            });

        let debug_init = [0u32, 0u32, 0u32];
        let debug = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxelizer.debug"),
                contents: bytemuck::cast_slice(&debug_init),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });

        DenseBuffers {
            triangles,
            tile_offsets,
            tri_indices,
            occupancy,
            owner,
            color,
            params: params_buf,
            brick_origins,
            debug,
            word_count,
            num_voxels,
        }
    }

    fn create_dense_params(
        &self,
        mesh: &MeshInput,
        grid: &VoxelGridSpec,
        tiles: &TileSpec,
        opts: &VoxelizeOpts,
    ) -> Params {
        Params {
            grid_dims: [grid.dims[0], grid.dims[1], grid.dims[2], 0],
            tile_dims: [tiles.tile_dims[0], tiles.tile_dims[1], tiles.tile_dims[2], 0],
            num_tiles_xyz: [tiles.num_tiles[0], tiles.num_tiles[1], tiles.num_tiles[2], 0],
            num_triangles: mesh.triangles.len() as u32,
            num_tiles: tiles.num_tiles_total(),
            tile_voxels: tiles.tile_dims[0] * tiles.tile_dims[1] * tiles.tile_dims[2],
            store_owner: u32::from(opts.store_owner),
            store_color: u32::from(opts.store_color),
            debug: 0,
            _pad0: [0, 0],
        }
    }

    fn create_dense_bind_group(&self, buffers: &DenseBuffers) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("voxelizer.bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffers.triangles.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffers.tile_offsets.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffers.tri_indices.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: buffers.occupancy.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: buffers.owner.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: buffers.color.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: buffers.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: buffers.brick_origins.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: buffers.debug.as_entire_binding(),
                },
            ],
        })
    }
}

// === Dispatch ===

impl GpuVoxelizer {
    fn dispatch_dense(&self, bind_group: &wgpu::BindGroup, tiles: &TileSpec) -> Result<(), String> {
        let num_tiles = tiles.num_tiles_total();
        let workgroups = (num_tiles + self.tiles_per_workgroup - 1) / self.tiles_per_workgroup;
        self.ensure_workgroups_fit(workgroups, "dense dispatch")?;

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("voxelizer.encoder"),
            });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("voxelizer.pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }

        self.queue.submit([encoder.finish()]);
        Ok(())
    }
}

// === Readback ===

impl GpuVoxelizer {
    async fn readback_dense(
        &self,
        buffers: &DenseBuffers,
        grid: &VoxelGridSpec,
        mesh: &MeshInput,
        tiles: &TileSpec,
        opts: &VoxelizeOpts,
    ) -> VoxelizationOutput {
        let read_occupancy = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.read_occupancy"),
            size: (buffers.word_count * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let read_owner = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.read_owner"),
            size: (buffers.num_voxels * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let read_color = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.read_color"),
            size: (buffers.num_voxels * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("voxelizer.readback"),
            });

        encoder.copy_buffer_to_buffer(
            &buffers.occupancy,
            0,
            &read_occupancy,
            0,
            read_occupancy.size(),
        );
        encoder.copy_buffer_to_buffer(&buffers.owner, 0, &read_owner, 0, read_owner.size());
        encoder.copy_buffer_to_buffer(&buffers.color, 0, &read_color, 0, read_color.size());

        self.queue.submit([encoder.finish()]);
        self.device.poll(wgpu::Maintain::Wait);

        let occupancy = map_buffer_u32(&read_occupancy, &self.device).await;
        let owner = map_buffer_u32(&read_owner, &self.device).await;
        let color = map_buffer_u32(&read_color, &self.device).await;

        VoxelizationOutput {
            occupancy,
            owner_id: if opts.store_owner { Some(owner) } else { None },
            color_rgba: if opts.store_color { Some(color) } else { None },
            stats: DispatchStats {
                triangles: mesh.triangles.len() as u32,
                tiles: tiles.num_tiles_total(),
                voxels: grid.num_voxels() as u32,
                gpu_time_ms: None,
            },
        }
    }
}
