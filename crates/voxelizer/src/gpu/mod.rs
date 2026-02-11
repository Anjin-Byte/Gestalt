//! GPU-accelerated voxelization using wgpu compute shaders.
//!
//! This module provides `GpuVoxelizer` for high-performance surface voxelization
//! with both dense and sparse output modes.

use bytemuck::{Pod, Zeroable};

use crate::core::VoxelizeOpts;

mod buffers;
mod compact_attrs;
mod compact_positions;
mod dense;
mod pipelines;
mod shaders;
mod sparse;

pub(crate) use buffers::{map_buffer_f32, map_buffer_u32};
use pipelines::create_pipelines;

// Re-export the Params type for use by dense/sparse modules
pub(crate) use self::params::Params;

mod params {
    use bytemuck::{Pod, Zeroable};

    #[repr(C)]
    #[derive(Clone, Copy, Pod, Zeroable)]
    pub struct Params {
        pub grid_dims: [u32; 4],
        pub tile_dims: [u32; 4],
        pub num_tiles_xyz: [u32; 4],
        pub num_triangles: u32,
        pub num_tiles: u32,
        pub tile_voxels: u32,
        pub store_owner: u32,
        pub store_color: u32,
        pub debug: u32,
        pub _pad0: [u32; 2],
    }
}

/// Configuration for the GPU voxelizer.
#[derive(Debug, Clone)]
pub struct GpuVoxelizerConfig {
    /// Workgroup size for compute shaders (0 = auto-detect).
    pub workgroup_size: u32,
    /// Number of tiles processed per workgroup.
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
pub(crate) struct CompactParams {
    pub brick_dim: u32,
    pub brick_count: u32,
    pub max_positions: u32,
    pub _pad0: u32,
    pub origin_world: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct CompactAttrsParams {
    pub brick_dim: u32,
    pub brick_count: u32,
    pub max_entries: u32,
    pub _pad0: u32,
    pub grid_dims: [u32; 4],
}

/// GPU-accelerated voxelizer using wgpu compute shaders.
///
/// Supports both dense voxelization (full grid) and sparse voxelization
/// (brick-based, only allocating storage for occupied regions).
pub struct GpuVoxelizer {
    pub(crate) instance: wgpu::Instance,
    pub(crate) adapter: wgpu::Adapter,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) pipeline: wgpu::ComputePipeline,
    pub(crate) bind_group_layout: wgpu::BindGroupLayout,
    pub(crate) compact_pipeline: wgpu::ComputePipeline,
    pub(crate) compact_bind_group_layout: wgpu::BindGroupLayout,
    pub(crate) compact_attrs_pipeline: wgpu::ComputePipeline,
    pub(crate) compact_attrs_bind_group_layout: wgpu::BindGroupLayout,
    pub(crate) workgroup_size: u32,
    pub(crate) tiles_per_workgroup: u32,
    pub(crate) max_invocations: u32,
    pub(crate) brick_dim: u32,
    pub(crate) max_storage_buffer_binding_size: u64,
    pub(crate) max_storage_buffers_per_shader_stage: u32,
    pub(crate) max_compute_workgroups_per_dimension: u32,
}

/// Summary of GPU device limits relevant to voxelization.
#[derive(Debug, Clone, Copy)]
pub struct GpuLimitsSummary {
    pub max_invocations_per_workgroup: u32,
    pub max_storage_buffers_per_shader_stage: u32,
    pub max_storage_buffer_binding_size: u64,
    pub max_compute_workgroups_per_dimension: u32,
}

impl GpuVoxelizer {
    /// Creates a new GPU voxelizer with the given configuration.
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

        let (workgroup_size, tiles_per_workgroup) =
            compute_workgroup_params(&config, max_invocations);

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

    /// Returns the wgpu instance.
    pub fn instance(&self) -> &wgpu::Instance {
        &self.instance
    }

    /// Returns the wgpu adapter.
    pub fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    /// Returns the wgpu device.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Returns the wgpu queue.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Returns the brick dimension used for sparse voxelization.
    pub fn brick_dim(&self) -> u32 {
        self.brick_dim
    }

    /// Returns a summary of GPU device limits.
    pub fn limits_summary(&self) -> GpuLimitsSummary {
        GpuLimitsSummary {
            max_invocations_per_workgroup: self.max_invocations,
            max_storage_buffers_per_shader_stage: self.max_storage_buffers_per_shader_stage,
            max_storage_buffer_binding_size: self.max_storage_buffer_binding_size,
            max_compute_workgroups_per_dimension: self.max_compute_workgroups_per_dimension,
        }
    }

    /// Returns the workgroup size used by compute shaders.
    pub fn workgroup_size(&self) -> u32 {
        self.workgroup_size
    }

    /// Validates that the workgroup count fits within device limits.
    pub(crate) fn ensure_workgroups_fit(&self, workgroups: u32, label: &str) -> Result<(), String> {
        if workgroups > self.max_compute_workgroups_per_dimension {
            return Err(format!(
                "{label}: workgroups {} exceed max {}",
                workgroups, self.max_compute_workgroups_per_dimension
            ));
        }
        Ok(())
    }

    /// Validates that a buffer size fits within device limits.
    pub(crate) fn ensure_storage_fits(&self, bytes: u64, label: &str) -> Result<(), String> {
        if bytes > self.max_storage_buffer_binding_size {
            return Err(format!(
                "{label}: buffer size {} bytes exceeds max {} bytes",
                bytes, self.max_storage_buffer_binding_size
            ));
        }
        Ok(())
    }

    /// Computes the maximum number of bricks that can be processed in one dispatch.
    pub(crate) fn max_bricks_per_dispatch(&self, brick_dim: u32, opts: &VoxelizeOpts) -> usize {
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

    /// Creates an empty position buffer (used for zero-result compaction).
    pub(crate) fn empty_position_buffer(&self) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voxelizer.compact.empty_positions"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }
}

/// Computes workgroup size and tiles per workgroup from config and device limits.
fn compute_workgroup_params(config: &GpuVoxelizerConfig, max_invocations: u32) -> (u32, u32) {
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

    (workgroup_size, tiles_per_workgroup)
}
