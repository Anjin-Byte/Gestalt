use bytemuck;
use wgpu::util::DeviceExt;

use crate::core::{DispatchStats, MeshInput, SparseVoxelizationOutput, VoxelGridSpec, VoxelizeOpts};
use crate::csr::{build_brick_csr, BrickTriangleCsr};

use super::buffers::map_buffer_u32;
use super::{GpuVoxelizer, Params};

impl GpuVoxelizer {
    pub async fn voxelize_surface_sparse(
        &self,
        mesh: &MeshInput,
        grid: &VoxelGridSpec,
        opts: &VoxelizeOpts,
    ) -> Result<SparseVoxelizationOutput, String> {
        grid.validate()?;
        mesh.validate()?;

        let brick_dim = self.brick_dim;
        let csr = build_brick_csr(mesh, grid, brick_dim, opts.epsilon);
        self.run_sparse(mesh, grid, opts, brick_dim, csr).await
    }

    pub async fn voxelize_surface_sparse_chunked(
        &self,
        mesh: &MeshInput,
        grid: &VoxelGridSpec,
        opts: &VoxelizeOpts,
        chunk_size: usize,
    ) -> Result<Vec<SparseVoxelizationOutput>, String> {
        grid.validate()?;
        mesh.validate()?;

        let brick_dim = self.brick_dim;
        let csr = build_brick_csr(mesh, grid, brick_dim, opts.epsilon);
        if csr.brick_origins.is_empty() {
            return Ok(Vec::new());
        }

        let mut chunks = Vec::new();
        let brick_count = csr.brick_origins.len();
        let max_bricks = self.max_bricks_per_dispatch(brick_dim, opts).min(brick_count);
        let requested = if chunk_size == 0 {
            max_bricks
        } else {
            chunk_size.min(max_bricks)
        };
        let chunk_size = requested.max(1);

        let mut start = 0usize;
        while start < brick_count {
            let end = (start + chunk_size).min(brick_count);
            let offset_start = csr.brick_offsets[start] as usize;
            let offset_end = csr.brick_offsets[end] as usize;
            let tri_indices = csr.tri_indices[offset_start..offset_end].to_vec();
            let mut brick_offsets = Vec::with_capacity(end - start + 1);
            let base = csr.brick_offsets[start];
            for idx in start..=end {
                brick_offsets.push(csr.brick_offsets[idx] - base);
            }
            let brick_origins = csr.brick_origins[start..end].to_vec();
            let sub = BrickTriangleCsr {
                brick_origins,
                brick_offsets,
                tri_indices,
            };
            let output = self.run_sparse(mesh, grid, opts, brick_dim, sub).await?;
            chunks.push(output);
            start = end;
        }
        Ok(chunks)
    }

    async fn run_sparse(
        &self,
        mesh: &MeshInput,
        grid: &VoxelGridSpec,
        opts: &VoxelizeOpts,
        brick_dim: u32,
        csr: BrickTriangleCsr,
    ) -> Result<SparseVoxelizationOutput, String> {
        let brick_voxels = (brick_dim * brick_dim * brick_dim) as usize;
        let words_per_brick = (brick_voxels + 31) / 32;
        let brick_count = csr.brick_origins.len() as u32;
        self.ensure_workgroups_fit(
            (brick_count + self.tiles_per_workgroup - 1) / self.tiles_per_workgroup,
            "sparse dispatch",
        )?;
        let occupancy_bytes =
            (words_per_brick as u64).saturating_mul(4).saturating_mul(brick_count as u64);
        self.ensure_storage_fits(occupancy_bytes, "sparse occupancy")?;
        if opts.store_owner {
            let owner_bytes =
                (brick_voxels as u64).saturating_mul(4).saturating_mul(brick_count as u64);
            self.ensure_storage_fits(owner_bytes, "sparse owner")?;
        }
        if opts.store_color {
            let color_bytes =
                (brick_voxels as u64).saturating_mul(4).saturating_mul(brick_count as u64);
            self.ensure_storage_fits(color_bytes, "sparse color")?;
        }

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

        let tri_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer_sparse.triangles"),
            contents: bytemuck::cast_slice(&tri_data),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let mut brick_origin_data = Vec::with_capacity(csr.brick_origins.len());
        for origin in &csr.brick_origins {
            brick_origin_data.push([origin[0], origin[1], origin[2], 0]);
        }
        let brick_origins_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer_sparse.brick_origins"),
            contents: bytemuck::cast_slice(&brick_origin_data),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let brick_offsets_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer_sparse.brick_offsets"),
            contents: bytemuck::cast_slice(&csr.brick_offsets),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let tri_indices_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer_sparse.tri_indices"),
            contents: bytemuck::cast_slice(&csr.tri_indices),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let occupancy_init = vec![0u32; words_per_brick * csr.brick_origins.len()];
        let occupancy_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer_sparse.occupancy"),
            contents: bytemuck::cast_slice(&occupancy_init),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });

        let owner_init = vec![u32::MAX; brick_voxels * csr.brick_origins.len()];
        let owner_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer_sparse.owner"),
            contents: bytemuck::cast_slice(&owner_init),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });

        let color_init = vec![0u32; brick_voxels * csr.brick_origins.len()];
        let color_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer_sparse.color"),
            contents: bytemuck::cast_slice(&color_init),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });

        let params = Params {
            grid_dims: [grid.dims[0], grid.dims[1], grid.dims[2], 0],
            tile_dims: [brick_dim, brick_dim, brick_dim, 0],
            num_tiles_xyz: [0, 0, 0, 0],
            num_triangles: mesh.triangles.len() as u32,
            num_tiles: brick_count,
            tile_voxels: brick_voxels as u32,
            store_owner: if opts.store_owner { 1 } else { 0 },
            store_color: if opts.store_color { 1 } else { 0 },
            debug: 1,
            _pad0: [0, 0],
        };
        let params_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer_sparse.params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let debug_init = [0u32, 0u32, 0u32];
        let debug_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer_sparse.debug"),
            contents: bytemuck::cast_slice(&debug_init),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("voxelizer_sparse.bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: tri_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: brick_offsets_buf.as_entire_binding() },
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
            label: Some("voxelizer_sparse.encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("voxelizer_sparse.pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            let workgroups =
                (brick_count + self.tiles_per_workgroup - 1) / self.tiles_per_workgroup;
            pass.dispatch_workgroups(workgroups, 1, 1);
        }

        let read_occupancy = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer_sparse.read_occupancy"),
            size: (occupancy_init.len() * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let read_owner = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer_sparse.read_owner"),
            size: (owner_init.len() * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let read_color = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer_sparse.read_color"),
            size: (color_init.len() * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let read_debug = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer_sparse.read_debug"),
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
        let debug = map_buffer_u32(&read_debug, &self.device).await;

        Ok(SparseVoxelizationOutput {
            brick_dim,
            brick_origins: csr.brick_origins,
            occupancy,
            owner_id: if opts.store_owner { Some(owner) } else { None },
            color_rgba: if opts.store_color { Some(color) } else { None },
            debug_flags: [0, 0, 0],
            debug_workgroups: *debug.get(0).unwrap_or(&0),
            debug_tested: *debug.get(1).unwrap_or(&0),
            debug_hits: *debug.get(2).unwrap_or(&0),
            stats: DispatchStats {
                triangles: mesh.triangles.len() as u32,
                tiles: brick_count,
                voxels: (grid.num_voxels()) as u32,
                gpu_time_ms: None,
            },
        })
    }
}
