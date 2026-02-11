//! Sparse voxelization - brick-based output for memory efficiency.

use bytemuck;
use wgpu::util::DeviceExt;

use crate::core::{DispatchStats, MeshInput, SparseVoxelizationOutput, VoxelGridSpec, VoxelizeOpts};
use crate::csr::{build_brick_csr, BrickTriangleCsr};

use super::map_buffer_u32;
use super::{GpuVoxelizer, Params};

impl GpuVoxelizer {
    /// Voxelizes a mesh surface into sparse brick-based output.
    ///
    /// Only allocates storage for bricks that contain geometry.
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

    /// Voxelizes in chunks to handle large meshes within GPU limits.
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

        let chunk_size = self.compute_chunk_size(brick_dim, opts, chunk_size, csr.brick_origins.len());
        self.process_chunks(mesh, grid, opts, brick_dim, &csr, chunk_size).await
    }

    async fn run_sparse(
        &self,
        mesh: &MeshInput,
        grid: &VoxelGridSpec,
        opts: &VoxelizeOpts,
        brick_dim: u32,
        csr: BrickTriangleCsr,
    ) -> Result<SparseVoxelizationOutput, String> {
        let brick_count = csr.brick_origins.len() as u32;

        self.validate_sparse_storage(brick_dim, brick_count, opts)?;

        let tri_data = prepare_sparse_triangle_data(mesh, grid);
        let buffers = self.create_sparse_buffers(brick_dim, opts, &csr, &tri_data);
        let params = self.create_sparse_params(mesh, grid, brick_dim, brick_count, opts);
        let bind_group = self.create_sparse_bind_group(&buffers, &params);

        self.dispatch_sparse(&bind_group, brick_count)?;

        let output = self
            .readback_sparse(&buffers, mesh, grid, brick_dim, csr.brick_origins, opts)
            .await;

        Ok(output)
    }
}

// === Chunked Processing ===

impl GpuVoxelizer {
    fn compute_chunk_size(
        &self,
        brick_dim: u32,
        opts: &VoxelizeOpts,
        requested: usize,
        total_bricks: usize,
    ) -> usize {
        let max_bricks = self.max_bricks_per_dispatch(brick_dim, opts).min(total_bricks);
        let chunk_size = if requested == 0 {
            max_bricks
        } else {
            requested.min(max_bricks)
        };
        chunk_size.max(1)
    }

    async fn process_chunks(
        &self,
        mesh: &MeshInput,
        grid: &VoxelGridSpec,
        opts: &VoxelizeOpts,
        brick_dim: u32,
        csr: &BrickTriangleCsr,
        chunk_size: usize,
    ) -> Result<Vec<SparseVoxelizationOutput>, String> {
        let mut chunks = Vec::new();
        let brick_count = csr.brick_origins.len();
        let mut start = 0usize;

        while start < brick_count {
            let end = (start + chunk_size).min(brick_count);
            let sub_csr = extract_chunk_csr(csr, start, end);
            let output = self.run_sparse(mesh, grid, opts, brick_dim, sub_csr).await?;
            chunks.push(output);
            start = end;
        }

        Ok(chunks)
    }
}

fn extract_chunk_csr(csr: &BrickTriangleCsr, start: usize, end: usize) -> BrickTriangleCsr {
    let offset_start = csr.brick_offsets[start] as usize;
    let offset_end = csr.brick_offsets[end] as usize;
    let tri_indices = csr.tri_indices[offset_start..offset_end].to_vec();

    let base = csr.brick_offsets[start];
    let brick_offsets: Vec<u32> = (start..=end)
        .map(|idx| csr.brick_offsets[idx] - base)
        .collect();

    let brick_origins = csr.brick_origins[start..end].to_vec();

    BrickTriangleCsr {
        brick_origins,
        brick_offsets,
        tri_indices,
    }
}

// === Validation ===

impl GpuVoxelizer {
    fn validate_sparse_storage(
        &self,
        brick_dim: u32,
        brick_count: u32,
        opts: &VoxelizeOpts,
    ) -> Result<(), String> {
        let workgroups = (brick_count + self.tiles_per_workgroup - 1) / self.tiles_per_workgroup;
        self.ensure_workgroups_fit(workgroups, "sparse dispatch")?;

        let brick_voxels = (brick_dim * brick_dim * brick_dim) as usize;
        let words_per_brick = (brick_voxels + 31) / 32;

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

        Ok(())
    }
}

// === Triangle Data ===

fn prepare_sparse_triangle_data(mesh: &MeshInput, grid: &VoxelGridSpec) -> Vec<[f32; 4]> {
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

struct SparseBuffers {
    triangles: wgpu::Buffer,
    brick_origins: wgpu::Buffer,
    brick_offsets: wgpu::Buffer,
    tri_indices: wgpu::Buffer,
    occupancy: wgpu::Buffer,
    owner: wgpu::Buffer,
    color: wgpu::Buffer,
    debug: wgpu::Buffer,
    occupancy_len: usize,
    owner_len: usize,
    color_len: usize,
}

impl GpuVoxelizer {
    fn create_sparse_buffers(
        &self,
        brick_dim: u32,
        _opts: &VoxelizeOpts,
        csr: &BrickTriangleCsr,
        tri_data: &[[f32; 4]],
    ) -> SparseBuffers {
        let brick_voxels = (brick_dim * brick_dim * brick_dim) as usize;
        let words_per_brick = (brick_voxels + 31) / 32;
        let brick_count = csr.brick_origins.len();

        let triangles = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxelizer_sparse.triangles"),
                contents: bytemuck::cast_slice(tri_data),
                usage: wgpu::BufferUsages::STORAGE,
            });

        let brick_origin_data: Vec<[u32; 4]> = csr
            .brick_origins
            .iter()
            .map(|o| [o[0], o[1], o[2], 0])
            .collect();

        let brick_origins = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxelizer_sparse.brick_origins"),
                contents: bytemuck::cast_slice(&brick_origin_data),
                usage: wgpu::BufferUsages::STORAGE,
            });

        let brick_offsets = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxelizer_sparse.brick_offsets"),
                contents: bytemuck::cast_slice(&csr.brick_offsets),
                usage: wgpu::BufferUsages::STORAGE,
            });

        let tri_indices = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxelizer_sparse.tri_indices"),
                contents: bytemuck::cast_slice(&csr.tri_indices),
                usage: wgpu::BufferUsages::STORAGE,
            });

        let occupancy_len = words_per_brick * brick_count;
        let occupancy_init = vec![0u32; occupancy_len];
        let occupancy = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxelizer_sparse.occupancy"),
                contents: bytemuck::cast_slice(&occupancy_init),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });

        let owner_len = brick_voxels * brick_count;
        let owner_init = vec![u32::MAX; owner_len];
        let owner = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxelizer_sparse.owner"),
                contents: bytemuck::cast_slice(&owner_init),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });

        let color_len = brick_voxels * brick_count;
        let color_init = vec![0u32; color_len];
        let color = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxelizer_sparse.color"),
                contents: bytemuck::cast_slice(&color_init),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });

        let debug_init = [0u32, 0u32, 0u32];
        let debug = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxelizer_sparse.debug"),
                contents: bytemuck::cast_slice(&debug_init),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });

        SparseBuffers {
            triangles,
            brick_origins,
            brick_offsets,
            tri_indices,
            occupancy,
            owner,
            color,
            debug,
            occupancy_len,
            owner_len,
            color_len,
        }
    }

    fn create_sparse_params(
        &self,
        mesh: &MeshInput,
        grid: &VoxelGridSpec,
        brick_dim: u32,
        brick_count: u32,
        opts: &VoxelizeOpts,
    ) -> wgpu::Buffer {
        let brick_voxels = brick_dim * brick_dim * brick_dim;

        let params = Params {
            grid_dims: [grid.dims[0], grid.dims[1], grid.dims[2], 0],
            tile_dims: [brick_dim, brick_dim, brick_dim, 0],
            num_tiles_xyz: [0, 0, 0, 0],
            num_triangles: mesh.triangles.len() as u32,
            num_tiles: brick_count,
            tile_voxels: brick_voxels,
            store_owner: u32::from(opts.store_owner),
            store_color: u32::from(opts.store_color),
            debug: 1,
            _pad0: [0, 0],
        };

        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxelizer_sparse.params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            })
    }

    fn create_sparse_bind_group(
        &self,
        buffers: &SparseBuffers,
        params: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("voxelizer_sparse.bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffers.triangles.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffers.brick_offsets.as_entire_binding(),
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
                    resource: params.as_entire_binding(),
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
    fn dispatch_sparse(&self, bind_group: &wgpu::BindGroup, brick_count: u32) -> Result<(), String> {
        let workgroups = (brick_count + self.tiles_per_workgroup - 1) / self.tiles_per_workgroup;

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("voxelizer_sparse.encoder"),
            });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("voxelizer_sparse.pass"),
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
    async fn readback_sparse(
        &self,
        buffers: &SparseBuffers,
        mesh: &MeshInput,
        grid: &VoxelGridSpec,
        brick_dim: u32,
        brick_origins: Vec<[u32; 3]>,
        opts: &VoxelizeOpts,
    ) -> SparseVoxelizationOutput {
        let read_occupancy = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer_sparse.read_occupancy"),
            size: (buffers.occupancy_len * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let read_owner = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer_sparse.read_owner"),
            size: (buffers.owner_len * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let read_color = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer_sparse.read_color"),
            size: (buffers.color_len * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let read_debug = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer_sparse.read_debug"),
            size: 12,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("voxelizer_sparse.readback"),
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
        encoder.copy_buffer_to_buffer(&buffers.debug, 0, &read_debug, 0, read_debug.size());

        self.queue.submit([encoder.finish()]);
        self.device.poll(wgpu::Maintain::Wait);

        let occupancy = map_buffer_u32(&read_occupancy, &self.device).await;
        let owner = map_buffer_u32(&read_owner, &self.device).await;
        let color = map_buffer_u32(&read_color, &self.device).await;
        let debug = map_buffer_u32(&read_debug, &self.device).await;

        let brick_count = brick_origins.len() as u32;

        SparseVoxelizationOutput {
            brick_dim,
            brick_origins,
            occupancy,
            owner_id: if opts.store_owner { Some(owner) } else { None },
            color_rgba: if opts.store_color { Some(color) } else { None },
            debug_flags: [0, 0, 0],
            debug_workgroups: *debug.first().unwrap_or(&0),
            debug_tested: *debug.get(1).unwrap_or(&0),
            debug_hits: *debug.get(2).unwrap_or(&0),
            stats: DispatchStats {
                triangles: mesh.triangles.len() as u32,
                tiles: brick_count,
                voxels: grid.num_voxels() as u32,
                gpu_time_ms: None,
            },
        }
    }
}
