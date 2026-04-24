//! v3 cascade build pipeline — Phase A.
//!
//! Owns the compute pipeline and bind group layouts for the v3 build pass.
//! Creates the WGSL shader module by string-prepending `dda_common.wgsl` and
//! `cascade_common.wgsl` to `cascade_build.wgsl`, the same way v2 does.
//!
//! Two bind groups:
//! - **Group 0** (world data): same shape as v2's group 0 — occupancy,
//!   flags, slot table, materials, palette, palette meta, index buf pool,
//!   slot table params. Sourced from the shared chunk pool, not copied.
//! - **Group 1** (v3 cascade data): payload SSBO (read_write), slot table,
//!   cascade params uniform, active probe chunks list. All compute-visible.

use crate::gi::v3::resources::V3CascadeResources;

/// v3 cascade build compute pipeline. Mirrors `passes::summary::SummaryPass`
/// but with two bind group layouts.
pub struct V3CascadePipeline {
    pipeline: wgpu::ComputePipeline,
    /// Group 0 layout — chunk pool world data. Same as v2's group 0 shape.
    pub world_data_layout: wgpu::BindGroupLayout,
    /// Group 1 layout — v3 cascade data. Different from the FRAGMENT-visible
    /// `V3CascadeResources::group3_layout` because the build pass needs
    /// `read_write` payload access and an extra `active_probe_chunks` binding.
    pub cascade_data_layout: wgpu::BindGroupLayout,
}

impl V3CascadePipeline {
    pub fn new(device: &wgpu::Device) -> Self {
        // String-prepend the helper shaders into the build kernel. WGSL has
        // no #include, so we concatenate at module-creation time. Same
        // technique as v2's CascadeBuildPass.
        let dda_common = include_str!("shaders/dda_common.wgsl");
        let cascade_common = include_str!("shaders/cascade_common.wgsl");
        let cascade_build = include_str!("shaders/cascade_build.wgsl");
        let combined = format!("{}\n{}\n{}", dda_common, cascade_common, cascade_build);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("v3-cascade-build-shader"),
            source: wgpu::ShaderSource::Wgsl(combined.into()),
        });

        // ── Group 0 — World data (same shape as v2) ───────────────
        let world_data_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("v3-cascade-world-data-layout"),
            entries: &[
                storage_read_entry(0), // occupancy
                storage_read_entry(1), // flags
                storage_read_entry(2), // slot_table
                storage_read_entry(3), // material_table
                storage_read_entry(4), // palette
                storage_read_entry(5), // palette_meta
                storage_read_entry(6), // index_buf_pool
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
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

        // ── Group 1 — v3 cascade data ─────────────────────────────
        let cascade_data_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("v3-cascade-data-layout"),
                entries: &[
                    // @binding(0) probe_payload (read_write storage)
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // @binding(1) probe_slot_table (read storage)
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
                    // @binding(2) cascade_params (uniform)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // @binding(3) active_probe_chunks (read storage)
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
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("v3-cascade-build-pipe-layout"),
            bind_group_layouts: &[Some(&world_data_layout), Some(&cascade_data_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("v3-cascade-build-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self {
            pipeline,
            world_data_layout,
            cascade_data_layout,
        }
    }

    /// Create a group 0 bind group from chunk pool buffers. Stable across
    /// frames; recreate only when the chunk pool buffers themselves change.
    pub fn create_world_data_bind_group(
        &self,
        device: &wgpu::Device,
        occupancy_buf: &wgpu::Buffer,
        flags_buf: &wgpu::Buffer,
        slot_table_buf: &wgpu::Buffer,
        material_table_buf: &wgpu::Buffer,
        palette_buf: &wgpu::Buffer,
        palette_meta_buf: &wgpu::Buffer,
        index_buf_pool_buf: &wgpu::Buffer,
        slot_table_params_buf: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("v3-cascade-world-data-bg"),
            layout: &self.world_data_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: occupancy_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: flags_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: slot_table_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: material_table_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: palette_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: palette_meta_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: index_buf_pool_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: slot_table_params_buf.as_entire_binding() },
            ],
        })
    }

    /// Create a group 1 bind group from v3 resources. Stable across frames;
    /// recreate only if `V3CascadeResources` is reallocated (which doesn't
    /// happen during normal operation — Phase A allocates once at startup).
    pub fn create_cascade_data_bind_group(
        &self,
        device: &wgpu::Device,
        resources: &V3CascadeResources,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("v3-cascade-data-bg"),
            layout: &self.cascade_data_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: resources.probe_payload_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: resources.probe_slot_table_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: resources.cascade_params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: resources.active_probe_chunks_buf.as_entire_binding(),
                },
            ],
        })
    }

    /// Record a v3 cascade build dispatch into a command encoder.
    ///
    /// Dispatch shape: `(16, 16, 16 * V3_MAX_PROBE_SLOTS)` workgroups,
    /// `(8, 8, 1)` threads per workgroup. Unallocated probe slots
    /// early-out via the `active_probe_chunks` sentinel.
    pub fn dispatch(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        world_bg: &wgpu::BindGroup,
        cascade_bg: &wgpu::BindGroup,
    ) {
        let probes_per_axis = crate::gi::v3::constants::V3_PROBES_PER_CHUNK_AXIS;
        let max_slots = crate::gi::v3::constants::V3_MAX_PROBE_SLOTS;
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("v3-cascade-build"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, Some(world_bg), &[]);
        pass.set_bind_group(1, Some(cascade_bg), &[]);
        pass.dispatch_workgroups(probes_per_axis, probes_per_axis, probes_per_axis * max_slots);
    }
}

fn storage_read_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
