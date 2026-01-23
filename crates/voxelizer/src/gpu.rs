use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::core::{
    DispatchStats, MeshInput, SparseVoxelizationOutput, TileSpec, VoxelGridSpec,
    VoxelizationOutput, VoxelizeOpts,
};

mod buffers;
mod dense;
mod pipelines;
mod shaders;
mod sparse;

use buffers::{map_buffer_f32, map_buffer_u32};
use pipelines::create_pipelines;

#[derive(Debug, Clone)]
pub struct GpuVoxelizerConfig {
    pub workgroup_size: u32,
    pub tiles_per_workgroup: u32,
}

impl Default for GpuVoxelizerConfig {
    fn default() -> Self {
        Self {
            workgroup_size: 0,
            tiles_per_workgroup: 2,
        }
    }
}

const MAX_TILES_PER_WORKGROUP: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    grid_dims: [u32; 4],
    tile_dims: [u32; 4],
    num_tiles_xyz: [u32; 4],
    num_triangles: u32,
    num_tiles: u32,
    tile_voxels: u32,
    store_owner: u32,
    store_color: u32,
    debug: u32,
    _pad0: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CompactParams {
    brick_dim: u32,
    brick_count: u32,
    max_positions: u32,
    _pad0: u32,
    origin_world: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CompactAttrsParams {
    brick_dim: u32,
    brick_count: u32,
    max_entries: u32,
    _pad0: u32,
    grid_dims: [u32; 4],
}

pub struct GpuVoxelizer {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    compact_pipeline: wgpu::ComputePipeline,
    compact_bind_group_layout: wgpu::BindGroupLayout,
    compact_attrs_pipeline: wgpu::ComputePipeline,
    compact_attrs_bind_group_layout: wgpu::BindGroupLayout,
    workgroup_size: u32,
    tiles_per_workgroup: u32,
    max_invocations: u32,
    brick_dim: u32,
    max_storage_buffer_binding_size: u64,
    max_storage_buffers_per_shader_stage: u32,
    max_compute_workgroups_per_dimension: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct GpuLimitsSummary {
    pub max_invocations_per_workgroup: u32,
    pub max_storage_buffers_per_shader_stage: u32,
    pub max_storage_buffer_binding_size: u64,
    pub max_compute_workgroups_per_dimension: u32,
}

impl GpuVoxelizer {
    pub async fn new(config: GpuVoxelizerConfig) -> Result<Self, String> {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok_or("No GPU adapter available")?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default(), None)
            .await
            .map_err(|e| format!("Failed to request device: {e}"))?;
        let limits = device.limits();
        let max_invocations = limits.max_compute_invocations_per_workgroup;
        let mut tiles_per_workgroup = config.tiles_per_workgroup.max(1);
        tiles_per_workgroup = tiles_per_workgroup.min(MAX_TILES_PER_WORKGROUP);
        let mut workgroup_size = if config.workgroup_size == 0 {
            let per_tile = max_invocations / tiles_per_workgroup;
            per_tile.clamp(32, max_invocations)
        } else {
            config.workgroup_size
        };
        if workgroup_size > max_invocations {
            workgroup_size = max_invocations;
        }
        if workgroup_size.saturating_mul(tiles_per_workgroup) > max_invocations {
            let max_tiles = (max_invocations / workgroup_size).max(1);
            tiles_per_workgroup = tiles_per_workgroup.min(max_tiles);
        }
        let max_storage_buffer_binding_size = limits.max_storage_buffer_binding_size as u64;
        let max_storage_buffers_per_shader_stage = limits.max_storage_buffers_per_shader_stage;
        let max_compute_workgroups_per_dimension = limits.max_compute_workgroups_per_dimension;
        let brick_dim = (max_invocations as f32).cbrt().floor() as u32;
        let brick_dim = brick_dim.clamp(2, 8);

        let pipelines = create_pipelines(&device, workgroup_size, tiles_per_workgroup).await?;

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            pipeline: pipelines.pipeline,
            bind_group_layout: pipelines.bind_group_layout,
            compact_pipeline: pipelines.compact_pipeline,
            compact_bind_group_layout: pipelines.compact_bind_group_layout,
            compact_attrs_pipeline: pipelines.compact_attrs_pipeline,
            compact_attrs_bind_group_layout: pipelines.compact_attrs_bind_group_layout,
            workgroup_size,
            tiles_per_workgroup,
            max_invocations,
            brick_dim,
            max_storage_buffer_binding_size,
            max_storage_buffers_per_shader_stage,
            max_compute_workgroups_per_dimension,
        })
    }

    pub fn instance(&self) -> &wgpu::Instance {
        &self.instance
    }

    pub fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub fn brick_dim(&self) -> u32 {
        self.brick_dim
    }

    pub fn limits_summary(&self) -> GpuLimitsSummary {
        GpuLimitsSummary {
            max_invocations_per_workgroup: self.max_invocations,
            max_storage_buffers_per_shader_stage: self.max_storage_buffers_per_shader_stage,
            max_storage_buffer_binding_size: self.max_storage_buffer_binding_size,
            max_compute_workgroups_per_dimension: self.max_compute_workgroups_per_dimension,
        }
    }

    pub fn workgroup_size(&self) -> u32 {
        self.workgroup_size
    }

    fn ensure_workgroups_fit(&self, workgroups: u32, label: &str) -> Result<(), String> {
        if workgroups > self.max_compute_workgroups_per_dimension {
            return Err(format!(
                "{label}: workgroups {} exceed max {}",
                workgroups, self.max_compute_workgroups_per_dimension
            ));
        }
        Ok(())
    }

    fn ensure_storage_fits(&self, bytes: u64, label: &str) -> Result<(), String> {
        if bytes > self.max_storage_buffer_binding_size {
            return Err(format!(
                "{label}: buffer size {} bytes exceeds max {} bytes",
                bytes, self.max_storage_buffer_binding_size
            ));
        }
        Ok(())
    }

    fn max_bricks_per_dispatch(&self, brick_dim: u32, opts: &VoxelizeOpts) -> usize {
        let brick_voxels = (brick_dim as u64)
            .saturating_mul(brick_dim as u64)
            .saturating_mul(brick_dim as u64);
        let words_per_brick = (brick_voxels + 31) / 32;
        let max_storage = self.max_storage_buffer_binding_size;
        let max_workgroups = self.max_compute_workgroups_per_dimension as u64;

        let occupancy_bytes = words_per_brick.saturating_mul(4);
        let mut max_bricks = if occupancy_bytes > 0 {
            max_storage / occupancy_bytes
        } else {
            max_storage
        };

        if opts.store_owner {
            let owner_bytes = brick_voxels.saturating_mul(4);
            if owner_bytes > 0 {
                max_bricks = max_bricks.min(max_storage / owner_bytes);
            }
        }
        if opts.store_color {
            let color_bytes = brick_voxels.saturating_mul(4);
            if color_bytes > 0 {
                max_bricks = max_bricks.min(max_storage / color_bytes);
            }
        }

        let max_bricks = max_bricks.min(max_workgroups).max(1);
        usize::try_from(max_bricks).unwrap_or(1)
    }

    pub async fn compact_sparse_positions(
        &self,
        occupancy: &[u32],
        brick_origins: &[[u32; 3]],
        brick_dim: u32,
        voxel_size: f32,
        origin_world: [f32; 3],
        max_positions: u32,
    ) -> Result<Vec<f32>, String> {
        let (buffer, count) = self
            .compact_sparse_positions_buffer(
                occupancy,
                brick_origins,
                brick_dim,
                voxel_size,
                origin_world,
                max_positions,
            )
            .await?;
        if count == 0 {
            return Ok(Vec::new());
        }
        let read_positions = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.compact.read_positions"),
            size: count as u64 * 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("voxelizer.compact.readback"),
        });
        encoder.copy_buffer_to_buffer(&buffer, 0, &read_positions, 0, read_positions.size());
        self.queue.submit([encoder.finish()]);
        self.device.poll(wgpu::Maintain::Wait);
        let data = map_buffer_f32(&read_positions, &self.device).await;
        let mut positions = Vec::with_capacity(count as usize * 3);
        for i in 0..count as usize {
            let base = i * 4;
            positions.push(data[base]);
            positions.push(data[base + 1]);
            positions.push(data[base + 2]);
        }
        Ok(positions)
    }

    pub async fn compact_sparse_positions_buffer(
        &self,
        occupancy: &[u32],
        brick_origins: &[[u32; 3]],
        brick_dim: u32,
        voxel_size: f32,
        origin_world: [f32; 3],
        max_positions: u32,
    ) -> Result<(wgpu::Buffer, u32), String> {
        if max_positions == 0 {
            return Ok((self.empty_position_buffer(), 0));
        }
        let brick_count = brick_origins.len() as u32;
        if brick_count == 0 || occupancy.is_empty() {
            return Ok((self.empty_position_buffer(), 0));
        }
        if brick_count > self.max_compute_workgroups_per_dimension {
            return Err(format!(
                "brick_count {} exceeds max workgroups {}",
                brick_count, self.max_compute_workgroups_per_dimension
            ));
        }

        let brick_voxels = brick_dim * brick_dim * brick_dim;
        let words_per_brick = (brick_voxels + 31) / 32;
        let expected_words = words_per_brick as usize * brick_origins.len();
        if occupancy.len() < expected_words {
            return Err("occupancy buffer too small for brick list".into());
        }
        let occupancy_bytes = (occupancy.len() as u64).saturating_mul(4);
        self.ensure_storage_fits(occupancy_bytes, "compact occupancy")?;
        let brick_origins_bytes = (brick_origins.len() as u64).saturating_mul(16);
        self.ensure_storage_fits(brick_origins_bytes, "compact brick origins")?;
        let out_positions_bytes = (max_positions as u64).saturating_mul(16);
        if out_positions_bytes == 0 {
            return Ok((self.empty_position_buffer(), 0));
        }
        self.ensure_storage_fits(out_positions_bytes, "compact positions")?;

        let mut brick_origin_data = Vec::with_capacity(brick_origins.len());
        for origin in brick_origins {
            brick_origin_data.push([origin[0], origin[1], origin[2], 0]);
        }

        let occupancy_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.compact.occupancy"),
            contents: bytemuck::cast_slice(occupancy),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let brick_origins_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.compact.brick_origins"),
            contents: bytemuck::cast_slice(&brick_origin_data),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let out_positions_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.compact.positions"),
            size: out_positions_bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::VERTEX,
            mapped_at_creation: false,
        });
        let counter_init = [0u32];
        let counter_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.compact.counter"),
            contents: bytemuck::cast_slice(&counter_init),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let debug_init = [0u32, 0u32];
        let debug_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.compact.debug"),
            contents: bytemuck::cast_slice(&debug_init),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });

        let params = CompactParams {
            brick_dim,
            brick_count,
            max_positions,
            origin_world: [origin_world[0], origin_world[1], origin_world[2], voxel_size],
            _pad0: 0,
        };
        let params_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.compact.params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("voxelizer.compact.bind_group"),
            layout: &self.compact_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: occupancy_buf.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: brick_origins_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: out_positions_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry { binding: 3, resource: counter_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: params_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: debug_buf.as_entire_binding() },
            ],
        });

        self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("voxelizer.compact.encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("voxelizer.compact.pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.compact_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(brick_count, 1, 1);
        }

        let read_counter = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.compact.read_counter"),
            size: 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let read_debug = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.compact.read_debug"),
            size: 8,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        encoder.copy_buffer_to_buffer(&counter_buf, 0, &read_counter, 0, read_counter.size());
        encoder.copy_buffer_to_buffer(&debug_buf, 0, &read_debug, 0, read_debug.size());
        self.queue.submit([encoder.finish()]);
        self.device.poll(wgpu::Maintain::Wait);
        if let Some(err) = self.device.pop_error_scope().await {
            return Err(format!("compact pass validation error: {err}"));
        }

        let counter = map_buffer_u32(&read_counter, &self.device).await;
        let count = counter.get(0).copied().unwrap_or(0).min(max_positions);
        let debug = map_buffer_u32(&read_debug, &self.device).await;
        if debug.get(0).copied().unwrap_or(0) == 0 {
            return Err(format!(
                "compact pass produced no workgroups (brick_count={}, max_workgroups={})",
                brick_count, self.max_compute_workgroups_per_dimension
            ));
        }
        if debug.get(1).copied().unwrap_or(0) == 0 {
            return Ok((self.empty_position_buffer(), 0));
        }
        Ok((out_positions_buf, count))
    }

    fn empty_position_buffer(&self) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.compact.empty_positions"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    pub async fn compact_sparse_attributes(
        &self,
        occupancy: &[u32],
        owner_id: &[u32],
        color_rgba: &[u32],
        brick_origins: &[[u32; 3]],
        brick_dim: u32,
        grid_dims: [u32; 3],
        max_entries: u32,
    ) -> Result<(Vec<u32>, Vec<u32>, Vec<u32>), String> {
        if max_entries == 0 {
            return Ok((Vec::new(), Vec::new(), Vec::new()));
        }
        let brick_count = brick_origins.len() as u32;
        if brick_count == 0 || occupancy.is_empty() {
            return Ok((Vec::new(), Vec::new(), Vec::new()));
        }
        if brick_count > self.max_compute_workgroups_per_dimension {
            return Err(format!(
                "brick_count {} exceeds max workgroups {}",
                brick_count, self.max_compute_workgroups_per_dimension
            ));
        }

        let brick_voxels = brick_dim * brick_dim * brick_dim;
        let words_per_brick = (brick_voxels + 31) / 32;
        let expected_words = words_per_brick as usize * brick_origins.len();
        if occupancy.len() < expected_words {
            return Err("occupancy buffer too small for brick list".into());
        }
        let expected_attrs = brick_voxels as usize * brick_origins.len();
        if owner_id.len() < expected_attrs || color_rgba.len() < expected_attrs {
            return Err("owner/color buffer too small for brick list".into());
        }
        let occupancy_bytes = (occupancy.len() as u64).saturating_mul(4);
        self.ensure_storage_fits(occupancy_bytes, "compact attrs occupancy")?;
        let brick_origins_bytes = (brick_origins.len() as u64).saturating_mul(16);
        self.ensure_storage_fits(brick_origins_bytes, "compact attrs brick origins")?;
        let owner_bytes = (owner_id.len() as u64).saturating_mul(4);
        self.ensure_storage_fits(owner_bytes, "compact attrs owner")?;
        let color_bytes = (color_rgba.len() as u64).saturating_mul(4);
        self.ensure_storage_fits(color_bytes, "compact attrs color")?;
        let out_entries_bytes = (max_entries as u64).saturating_mul(4);
        if out_entries_bytes == 0 {
            return Ok((Vec::new(), Vec::new(), Vec::new()));
        }
        self.ensure_storage_fits(out_entries_bytes, "compact attrs out_indices")?;

        let mut brick_origin_data = Vec::with_capacity(brick_origins.len());
        for origin in brick_origins {
            brick_origin_data.push([origin[0], origin[1], origin[2], 0]);
        }

        let occupancy_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.compact_attrs.occupancy"),
            contents: bytemuck::cast_slice(occupancy),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let brick_origins_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.compact_attrs.brick_origins"),
            contents: bytemuck::cast_slice(&brick_origin_data),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let owner_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.compact_attrs.owner"),
            contents: bytemuck::cast_slice(owner_id),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let color_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.compact_attrs.color"),
            contents: bytemuck::cast_slice(color_rgba),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let out_indices_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.compact_attrs.indices"),
            size: out_entries_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let out_owner_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.compact_attrs.out_owner"),
            size: out_entries_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let out_color_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.compact_attrs.out_color"),
            size: out_entries_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let counter_init = [0u32];
        let counter_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.compact_attrs.counter"),
            contents: bytemuck::cast_slice(&counter_init),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });

        let params = CompactAttrsParams {
            brick_dim,
            brick_count,
            max_entries,
            _pad0: 0,
            grid_dims: [grid_dims[0], grid_dims[1], grid_dims[2], 0],
        };
        let params_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelizer.compact_attrs.params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("voxelizer.compact_attrs.bind_group"),
            layout: &self.compact_attrs_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: occupancy_buf.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: brick_origins_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry { binding: 2, resource: owner_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: color_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: out_indices_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: out_owner_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: out_color_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: counter_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: params_buf.as_entire_binding() },
            ],
        });

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("voxelizer.compact_attrs.encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("voxelizer.compact_attrs.pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.compact_attrs_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(brick_count, 1, 1);
        }
        let read_counter = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.compact_attrs.read_counter"),
            size: 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let read_indices = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.compact_attrs.read_indices"),
            size: max_entries as u64 * 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let read_owner = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.compact_attrs.read_owner"),
            size: max_entries as u64 * 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let read_color = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.compact_attrs.read_color"),
            size: max_entries as u64 * 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        encoder.copy_buffer_to_buffer(&counter_buf, 0, &read_counter, 0, read_counter.size());
        encoder.copy_buffer_to_buffer(&out_indices_buf, 0, &read_indices, 0, read_indices.size());
        encoder.copy_buffer_to_buffer(&out_owner_buf, 0, &read_owner, 0, read_owner.size());
        encoder.copy_buffer_to_buffer(&out_color_buf, 0, &read_color, 0, read_color.size());
        self.queue.submit([encoder.finish()]);
        self.device.poll(wgpu::Maintain::Wait);

        let counter = map_buffer_u32(&read_counter, &self.device).await;
        let count = counter.get(0).copied().unwrap_or(0).min(max_entries) as usize;
        if count == 0 {
            return Ok((Vec::new(), Vec::new(), Vec::new()));
        }
        let indices = map_buffer_u32(&read_indices, &self.device).await;
        let owners = map_buffer_u32(&read_owner, &self.device).await;
        let colors = map_buffer_u32(&read_color, &self.device).await;
        Ok((
            indices[..count].to_vec(),
            owners[..count].to_vec(),
            colors[..count].to_vec(),
        ))
    }


}
