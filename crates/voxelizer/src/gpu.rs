use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use std::collections::HashMap;

use crate::core::{
    DispatchStats, MeshInput, SparseVoxelizationOutput, TileSpec, VoxelGridSpec,
    VoxelizationOutput, VoxelizeOpts,
};
use crate::csr::{build_brick_csr, BrickTriangleCsr, build_tile_csr};

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

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("voxelizer.wgsl"),
            source: wgpu::ShaderSource::Wgsl(VOXELIZER_WGSL.into()),
        });
        let compact_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("voxelizer.compact.wgsl"),
            source: wgpu::ShaderSource::Wgsl(COMPACT_WGSL.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("voxelizer.bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let compact_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("voxelizer.compact_bind_group_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let compact_attrs_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("voxelizer.compact_attrs_bind_group_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 6,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 7,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 8,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("voxelizer.pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let compact_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("voxelizer.compact_pipeline_layout"),
                bind_group_layouts: &[&compact_bind_group_layout],
                push_constant_ranges: &[],
            });
        let compact_attrs_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("voxelizer.compact_attrs_pipeline_layout"),
                bind_group_layouts: &[&compact_attrs_bind_group_layout],
                push_constant_ranges: &[],
            });

        let mut constants = HashMap::new();
        constants.insert("WORKGROUP_SIZE".to_string(), workgroup_size as f64);
        constants.insert(
            "TILES_PER_WORKGROUP".to_string(),
            tiles_per_workgroup as f64,
        );
        let compilation_options = wgpu::PipelineCompilationOptions {
            constants: &constants,
            ..Default::default()
        };
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("voxelizer.pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: "main",
            compilation_options,
            cache: None,
        });
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let compact_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("voxelizer.compact_pipeline"),
            layout: Some(&compact_pipeline_layout),
            module: &compact_shader,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let compact_attrs_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("voxelizer.compact_attrs.wgsl"),
            source: wgpu::ShaderSource::Wgsl(COMPACT_ATTRS_WGSL.into()),
        });
        let compact_attrs_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("voxelizer.compact_attrs_pipeline"),
                layout: Some(&compact_attrs_pipeline_layout),
                module: &compact_attrs_shader,
                entry_point: "main",
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });
        if let Some(err) = device.pop_error_scope().await {
            return Err(format!("Compact pipeline validation error: {err}"));
        }

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            pipeline,
            bind_group_layout,
            compact_pipeline,
            compact_bind_group_layout,
            compact_attrs_pipeline,
            compact_attrs_bind_group_layout,
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

async fn map_buffer_u32(buffer: &wgpu::Buffer, device: &wgpu::Device) -> Vec<u32> {
    let slice = buffer.slice(..);
    let (sender, receiver) = futures::channel::oneshot::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device.poll(wgpu::Maintain::Wait);
    receiver.await.expect("map buffer").expect("map buffer");
    let data = slice.get_mapped_range();
    let result = bytemuck::cast_slice(&data).to_vec();
    drop(data);
    buffer.unmap();
    result
}

async fn map_buffer_f32(buffer: &wgpu::Buffer, device: &wgpu::Device) -> Vec<f32> {
    let slice = buffer.slice(..);
    let (sender, receiver) = futures::channel::oneshot::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device.poll(wgpu::Maintain::Wait);
    receiver.await.expect("map buffer").expect("map buffer");
    let data = slice.get_mapped_range();
    let result = bytemuck::cast_slice(&data).to_vec();
    drop(data);
    buffer.unmap();
    result
}

const VOXELIZER_WGSL: &str = r#"
struct Params {
  grid_dims: vec4<u32>,
  tile_dims: vec4<u32>,
  num_tiles_xyz: vec4<u32>,
  num_triangles: u32,
  num_tiles: u32,
  tile_voxels: u32,
  store_owner: u32,
  store_color: u32,
  debug: u32,
  _pad0: vec2<u32>,
};

@group(0) @binding(0) var<storage, read> tris: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> tile_offsets: array<u32>;
@group(0) @binding(4) var<storage, read> tri_indices: array<u32>;
@group(0) @binding(6) var<storage, read_write> occupancy: array<atomic<u32>>;
@group(0) @binding(7) var<storage, read_write> owner_id: array<u32>;
@group(0) @binding(8) var<storage, read_write> color_rgba: array<u32>;
@group(0) @binding(9) var<uniform> params: Params;
@group(0) @binding(10) var<storage, read> brick_origins: array<vec4<u32>>;
@group(0) @binding(11) var<storage, read_write> debug_counts: array<atomic<u32>>;

override WORKGROUP_SIZE: u32 = 64u;
override TILES_PER_WORKGROUP: u32 = 1u;
const TRI_STRIDE: u32 = 6u;
const MAX_ACTIVE_TRIS: u32 = 256u;
const MAX_TILES_PER_WORKGROUP: u32 = 4u;
var<workgroup> active_tris: array<u32, MAX_ACTIVE_TRIS * MAX_TILES_PER_WORKGROUP>;
var<workgroup> active_count: array<u32, MAX_TILES_PER_WORKGROUP>;
var<workgroup> active_overflow: array<u32, MAX_TILES_PER_WORKGROUP>;

fn hash_color(id: u32) -> u32 {
  var x = id * 1664525u + 1013904223u;
  let r = x & 255u;
  x = x * 1664525u + 1013904223u;
  let g = x & 255u;
  x = x * 1664525u + 1013904223u;
  let b = x & 255u;
  return r | (g << 8u) | (b << 16u) | (255u << 24u);
}

fn axis_test(axis: vec3<f32>, v0: vec3<f32>, v1: vec3<f32>, v2: vec3<f32>, half: vec3<f32>) -> bool {
  let p0 = dot(v0, axis);
  let p1 = dot(v1, axis);
  let p2 = dot(v2, axis);
  let min_p = min(p0, min(p1, p2));
  let max_p = max(p0, max(p1, p2));
  let r = half.x * abs(axis.x) + half.y * abs(axis.y) + half.z * abs(axis.z);
  return !(min_p > r || max_p < -r);
}

fn plane_box_intersects(normal: vec3<f32>, d: f32, center: vec3<f32>, half: vec3<f32>) -> bool {
  let r = half.x * abs(normal.x) + half.y * abs(normal.y) + half.z * abs(normal.z);
  let s = dot(normal, center) + d;
  return abs(s) <= r;
}

fn triangle_box_overlap(center: vec3<f32>, half: vec3<f32>, a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, normal: vec3<f32>, d: f32, tri_min: vec3<f32>, tri_max: vec3<f32>) -> bool {
  let v0 = a - center;
  let v1 = b - center;
  let v2 = c - center;
  let e0 = v1 - v0;
  let e1 = v2 - v1;
  let e2 = v0 - v2;

  // Fast AABB reject (triangle AABB vs box).
  let box_min = center - half;
  let box_max = center + half;
  if (tri_min.x > box_max.x || tri_max.x < box_min.x) {
    return false;
  }
  if (tri_min.y > box_max.y || tri_max.y < box_min.y) {
    return false;
  }
  if (tri_min.z > box_max.z || tri_max.z < box_min.z) {
    return false;
  }

  // Plane test before edge axes to early reject (precomputed plane).
  if (!plane_box_intersects(normal, d, center, half)) {
    return false;
  }

  let axes = array<vec3<f32>, 9>(
    vec3<f32>(0.0, -e0.z, e0.y),
    vec3<f32>(0.0, -e1.z, e1.y),
    vec3<f32>(0.0, -e2.z, e2.y),
    vec3<f32>(e0.z, 0.0, -e0.x),
    vec3<f32>(e1.z, 0.0, -e1.x),
    vec3<f32>(e2.z, 0.0, -e2.x),
    vec3<f32>(-e0.y, e0.x, 0.0),
    vec3<f32>(-e1.y, e1.x, 0.0),
    vec3<f32>(-e2.y, e2.x, 0.0)
  );

  for (var i = 0u; i < 9u; i = i + 1u) {
    if (!axis_test(axes[i], v0, v1, v2, half)) {
      return false;
    }
  }

  return true;
}

@compute @workgroup_size(WORKGROUP_SIZE, TILES_PER_WORKGROUP, 1)
fn main(@builtin(workgroup_id) wg_id: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
  let tile_lane = lid.y;
  let tile_index = wg_id.x * TILES_PER_WORKGROUP + tile_lane;
  let valid_tile = tile_index < params.num_tiles;
  if (lid.x == 0u && valid_tile && params.debug != 0u) {
    atomicAdd(&debug_counts[0], 1u);
  }

  var tile_min = vec3<u32>(0u, 0u, 0u);
  if (valid_tile && params.num_tiles_xyz.x > 0u) {
    let tile_x = tile_index % params.num_tiles_xyz.x;
    let tile_y = (tile_index / params.num_tiles_xyz.x) % params.num_tiles_xyz.y;
    let tile_z = tile_index / (params.num_tiles_xyz.x * params.num_tiles_xyz.y);
    tile_min = vec3<u32>(
      tile_x * params.tile_dims.x,
      tile_y * params.tile_dims.y,
      tile_z * params.tile_dims.z
    );
  } else if (valid_tile) {
    tile_min = brick_origins[tile_index].xyz;
  }

  var offset = 0u;
  var end = 0u;
  if (valid_tile) {
    offset = tile_offsets[tile_index];
    end = tile_offsets[tile_index + 1u];
  }
  let has_tris = valid_tile && (offset != end);

  let tile_max = vec3<u32>(
    min(tile_min.x + params.tile_dims.x, params.grid_dims.x),
    min(tile_min.y + params.tile_dims.y, params.grid_dims.y),
    min(tile_min.z + params.tile_dims.z, params.grid_dims.z)
  );
  let tile_min_f = vec3<f32>(f32(tile_min.x), f32(tile_min.y), f32(tile_min.z));
  let tile_max_f = vec3<f32>(f32(tile_max.x), f32(tile_max.y), f32(tile_max.z));
  let tile_center = (tile_min_f + tile_max_f) * 0.5;
  let tile_half = (tile_max_f - tile_min_f) * 0.5;

  if (lid.x == 0u) {
    active_count[tile_lane] = 0u;
    active_overflow[tile_lane] = 0u;
    if (has_tris) {
      let base_index = tile_lane * MAX_ACTIVE_TRIS;
      for (var i = offset; i < end; i = i + 1u) {
        let tri = tri_indices[i];
        if (tri >= params.num_triangles) {
          continue;
        }
        let base = tri * TRI_STRIDE;
        let plane = tris[base + 5u];
        if (plane_box_intersects(plane.xyz, plane.w, tile_center, tile_half)) {
          if (active_count[tile_lane] < MAX_ACTIVE_TRIS) {
            active_tris[base_index + active_count[tile_lane]] = tri;
            active_count[tile_lane] = active_count[tile_lane] + 1u;
          } else {
            active_overflow[tile_lane] = 1u;
          }
        }
      }
    }
  }
  workgroupBarrier();
  let half = vec3<f32>(0.5, 0.5, 0.5);

  let tile_voxels = params.tile_voxels;
  if (has_tris) {
    var linear = lid.x;
    loop {
      if (linear >= tile_voxels) {
        break;
      }
      let vx = linear % params.tile_dims.x;
      let vy = (linear / params.tile_dims.x) % params.tile_dims.y;
      let vz = (linear / (params.tile_dims.x * params.tile_dims.y));
      let gx = tile_min.x + vx;
      let gy = tile_min.y + vy;
      let gz = tile_min.z + vz;

      if (gx < params.grid_dims.x && gy < params.grid_dims.y && gz < params.grid_dims.z) {
        let center = vec3<f32>(f32(gx) + 0.5, f32(gy) + 0.5, f32(gz) + 0.5);
        var hit = false;
        var best = 0xffffffffu;
        if (active_overflow[tile_lane] == 0u) {
          let base_index = tile_lane * MAX_ACTIVE_TRIS;
          for (var i = 0u; i < active_count[tile_lane]; i = i + 1u) {
            let tri = active_tris[base_index + i];
            let base = tri * TRI_STRIDE;
            let a = tris[base].xyz;
            let b = tris[base + 1u].xyz;
            let c = tris[base + 2u].xyz;
            let tri_min = tris[base + 3u].xyz;
            let tri_max = tris[base + 4u].xyz;
            let plane = tris[base + 5u];
            if (triangle_box_overlap(center, half, a, b, c, plane.xyz, plane.w, tri_min, tri_max)) {
              hit = true;
              if (tri < best) {
                best = tri;
              }
            }
          }
        } else {
          for (var i = offset; i < end; i = i + 1u) {
            let tri = tri_indices[i];
            if (tri >= params.num_triangles) {
              continue;
            }
            let base = tri * TRI_STRIDE;
            let a = tris[base].xyz;
            let b = tris[base + 1u].xyz;
            let c = tris[base + 2u].xyz;
            let tri_min = tris[base + 3u].xyz;
            let tri_max = tris[base + 4u].xyz;
            let plane = tris[base + 5u];
            if (triangle_box_overlap(center, half, a, b, c, plane.xyz, plane.w, tri_min, tri_max)) {
              hit = true;
              if (tri < best) {
                best = tri;
              }
            }
          }
        }

        if (params.debug != 0u) {
          atomicAdd(&debug_counts[1], 1u);
        }
        if (hit) {
          if (params.debug != 0u) {
            atomicAdd(&debug_counts[2], 1u);
          }
          if (params.num_tiles_xyz.x > 0u) {
            let linear_index = gx + params.grid_dims.x * (gy + params.grid_dims.y * gz);
            let word = linear_index >> 5u;
            let bit = linear_index & 31u;
            atomicOr(&occupancy[word], 1u << bit);
            if (params.store_owner == 1u) {
              owner_id[linear_index] = best;
            }
            if (params.store_color == 1u) {
              color_rgba[linear_index] = hash_color(best);
            }
          } else {
            let local_index = vx + params.tile_dims.x * (vy + params.tile_dims.y * vz);
            let word = (tile_index * ((params.tile_voxels + 31u) / 32u)) + (local_index >> 5u);
            let bit = local_index & 31u;
            atomicOr(&occupancy[word], 1u << bit);
            if (params.store_owner == 1u) {
              owner_id[tile_index * params.tile_voxels + local_index] = best;
            }
            if (params.store_color == 1u) {
              color_rgba[tile_index * params.tile_voxels + local_index] = hash_color(best);
            }
          }
        }
      }
      linear = linear + WORKGROUP_SIZE;
    }
  }
}
"#;

const COMPACT_WGSL: &str = r#"
struct CompactParams {
  brick_dim: u32,
  brick_count: u32,
  max_positions: u32,
  _pad0: u32,
  origin_world: vec4<f32>,
};

@group(0) @binding(0) var<storage, read> occupancy: array<u32>;
@group(0) @binding(1) var<storage, read> brick_origins: array<vec4<u32>>;
@group(0) @binding(2) var<storage, read_write> out_positions: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> counter: array<atomic<u32>>;
@group(0) @binding(4) var<uniform> params: CompactParams;
@group(0) @binding(5) var<storage, read_write> debug: array<atomic<u32>>;

@compute @workgroup_size(64)
fn main(@builtin(workgroup_id) wg_id: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
  let brick_index = wg_id.x;
  if (brick_index >= params.brick_count) {
    return;
  }
  if (lid.x == 0u) {
    atomicAdd(&debug[0], 1u);
  }
  let brick_dim = params.brick_dim;
  let brick_voxels = brick_dim * brick_dim * brick_dim;
  let words_per_brick = (brick_voxels + 31u) / 32u;
  let base_word = brick_index * words_per_brick;
  let origin = brick_origins[brick_index].xyz;

  var linear = lid.x;
  loop {
    if (linear >= brick_voxels) {
      break;
    }
    let word = base_word + (linear >> 5u);
    let bit = linear & 31u;
    let mask = 1u << bit;
    if ((occupancy[word] & mask) != 0u) {
      atomicAdd(&debug[1], 1u);
      let idx = atomicAdd(&counter[0], 1u);
      if (idx < params.max_positions) {
        let vx = linear % brick_dim;
        let vy = (linear / brick_dim) % brick_dim;
        let vz = linear / (brick_dim * brick_dim);
        let gx = f32(origin.x + vx) + 0.5;
        let gy = f32(origin.y + vy) + 0.5;
        let gz = f32(origin.z + vz) + 0.5;
        let world = params.origin_world.xyz + vec3<f32>(gx, gy, gz) * params.origin_world.w;
        out_positions[idx] = vec4<f32>(world, 1.0);
      }
    }
    linear = linear + 64u;
  }
}
"#;

const COMPACT_ATTRS_WGSL: &str = r#"
struct CompactAttrsParams {
  brick_dim: u32,
  brick_count: u32,
  max_entries: u32,
  _pad0: u32,
  grid_dims: vec4<u32>,
};

@group(0) @binding(0) var<storage, read> occupancy: array<u32>;
@group(0) @binding(1) var<storage, read> brick_origins: array<vec4<u32>>;
@group(0) @binding(2) var<storage, read> owner_id: array<u32>;
@group(0) @binding(3) var<storage, read> color_rgba: array<u32>;
@group(0) @binding(4) var<storage, read_write> out_indices: array<u32>;
@group(0) @binding(5) var<storage, read_write> out_owner: array<u32>;
@group(0) @binding(6) var<storage, read_write> out_color: array<u32>;
@group(0) @binding(7) var<storage, read_write> counter: array<atomic<u32>>;
@group(0) @binding(8) var<uniform> params: CompactAttrsParams;

@compute @workgroup_size(64)
fn main(@builtin(workgroup_id) wg_id: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
  let brick_index = wg_id.x;
  if (brick_index >= params.brick_count) {
    return;
  }
  let brick_dim = params.brick_dim;
  let brick_voxels = brick_dim * brick_dim * brick_dim;
  let words_per_brick = (brick_voxels + 31u) / 32u;
  let base_word = brick_index * words_per_brick;
  let origin = brick_origins[brick_index].xyz;

  var linear = lid.x;
  loop {
    if (linear >= brick_voxels) {
      break;
    }
    let word = base_word + (linear >> 5u);
    let bit = linear & 31u;
    let mask = 1u << bit;
    if ((occupancy[word] & mask) != 0u) {
      let idx = atomicAdd(&counter[0], 1u);
      if (idx < params.max_entries) {
        let vx = linear % brick_dim;
        let vy = (linear / brick_dim) % brick_dim;
        let vz = linear / (brick_dim * brick_dim);
        let gx = origin.x + vx;
        let gy = origin.y + vy;
        let gz = origin.z + vz;
        let linear_index =
          gx + params.grid_dims.x * (gy + params.grid_dims.y * gz);
        let attr_index = brick_index * brick_voxels + linear;
        out_indices[idx] = linear_index;
        out_owner[idx] = owner_id[attr_index];
        out_color[idx] = color_rgba[attr_index];
      }
    }
    linear = linear + 64u;
  }
}
"#;
