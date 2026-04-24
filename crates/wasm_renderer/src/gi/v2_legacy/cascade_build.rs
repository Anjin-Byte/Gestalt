// REMOVE_WITH_V2 FILE: deleted at v3 completion alongside the rest of gi/v2_legacy/.
//! R-6 Cascade Build v2 — single-pass cast + inline merge.
//!
//! Each dispatch handles one cascade level. Uses ping-pong atlases:
//! reads coarser cascade from atlas_read, writes merged result to atlas_write.
//! Dynamic uniform buffer offset delivers per-cascade parameters.
//!
//! See: docs/Resident Representation/radiance-cascades-v2.md

/// GPU compute pass for radiance cascade build.
pub struct CascadeBuildPass {
    pipeline: wgpu::ComputePipeline,
    world_data_layout: wgpu::BindGroupLayout,
    per_pass_layout: wgpu::BindGroupLayout,
}

impl CascadeBuildPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let dda_common = include_str!("shaders/dda_common.wgsl");
        let cascade_src = include_str!("shaders/cascade_build.wgsl");
        let combined = format!("{}\n{}", dda_common, cascade_src);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cascade-build-v2-shader"),
            source: wgpu::ShaderSource::Wgsl(combined.into()),
        });

        // Group 0 — World Data (8 bindings, unchanged from v1)
        let world_data_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cascade-world-data-layout"),
                entries: &[
                    storage_read_entry(0),  // occupancy_atlas
                    storage_read_entry(1),  // flags
                    storage_read_entry(2),  // slot_table
                    storage_read_entry(3),  // material_table
                    storage_read_entry(4),  // palette
                    storage_read_entry(5),  // palette_meta
                    storage_read_entry(6),  // index_buf_pool
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

        // Group 1 — Per-Pass (4 bindings, dynamic offset on uniforms)
        let per_pass_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cascade-per-pass-v2-layout"),
                entries: &[
                    // @binding(0) cascade_uniforms (dynamic offset)
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // @binding(1) depth_texture
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // @binding(2) cascade_write (storage texture write)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: wgpu::TextureFormat::Rgba16Float,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    // @binding(3) cascade_read (sampled texture — coarser cascade)
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // @binding(4) normal_tex (G-buffer normals)
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // @binding(5) cascade_prev (previous frame's cascade 0 for multi-bounce)
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cascade-build-v2-pipeline-layout"),
            bind_group_layouts: &[Some(&world_data_layout), Some(&per_pass_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("cascade-build-v2-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self {
            pipeline,
            world_data_layout,
            per_pass_layout,
        }
    }

    /// Create the world data bind group (group 0). Stable across frames.
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
            label: Some("cascade-world-data-bg"),
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

    /// Create a per-pass bind group (group 1) for ping-pong dispatch.
    /// `cascade_write_view` is the atlas being written, `cascade_read_view` is the other atlas (read).
    pub fn create_per_pass_bind_group(
        &self,
        device: &wgpu::Device,
        cascade_uniforms_buf: &wgpu::Buffer,
        depth_view: &wgpu::TextureView,
        cascade_write_view: &wgpu::TextureView,
        cascade_read_view: &wgpu::TextureView,
        normal_view: &wgpu::TextureView,
        cascade_prev_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cascade-per-pass-v2-bg"),
            layout: &self.per_pass_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: cascade_uniforms_buf,
                        offset: 0,
                        size: Some(std::num::NonZeroU64::new(
                            std::mem::size_of::<super::backend::CascadeUniforms>() as u64
                        ).unwrap()),
                    }),
                },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(depth_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(cascade_write_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(cascade_read_view) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(normal_view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(cascade_prev_view) },
            ],
        })
    }

    pub fn pipeline(&self) -> &wgpu::ComputePipeline { &self.pipeline }
}

// Helper for storage buffer read-only entries
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
