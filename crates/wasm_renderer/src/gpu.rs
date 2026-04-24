//! GPU resource helpers — texture creation, pipeline creation, camera uniform.

/// Create a depth texture + view. Returns both so the texture can be
/// referenced by the Hi-Z pass (needs TEXTURE_BINDING).
pub fn create_depth_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth-texture"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Create a normal G-buffer texture (rgba16float) for storing world-space normals.
/// Written during the depth prepass, read by the cascade build shader.
pub fn create_normal_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("normal-gbuffer"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Create the Hi-Z pyramid texture with a full mip chain (r32float).
/// Returns the texture, the full-mip view, per-mip views for storage writes,
/// and the mip count.
pub fn create_hiz_pyramid(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, Vec<wgpu::TextureView>, u32) {
    let w = width.max(1);
    let h = height.max(1);
    let mip_count = (w.max(h) as f32).log2().floor() as u32 + 1;

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hiz-pyramid"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: mip_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });

    let mut mip_views = Vec::with_capacity(mip_count as usize);
    for mip in 0..mip_count {
        mip_views.push(texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some(&format!("hiz-mip-{mip}")),
            base_mip_level: mip,
            mip_level_count: Some(1),
            ..Default::default()
        }));
    }

    (texture, mip_views, mip_count)
}

// v2 cascade constants (N_CASCADES, CASCADE_UNIFORM_ALIGNED_SIZE,
// CascadeUniforms, create_cascade_atlas) moved to gi/v2_legacy/backend.rs.

/// Camera uniform data matching the WGSL `Camera` struct.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraUniform {
    pub view_proj: [f32; 16],  // mat4x4f
    pub position: [f32; 4],    // vec4f (xyz + padding)
}

/// Render resources: camera uniform buffer + bind groups + pipeline.
pub struct RenderResources {
    pub camera_buf: wgpu::Buffer,
    pub camera_layout: wgpu::BindGroupLayout,
    pub camera_bind_group: wgpu::BindGroup,
    pub vertex_layout: wgpu::BindGroupLayout,
    pub vertex_bind_group: wgpu::BindGroup,
    pub depth_pipeline: wgpu::RenderPipeline,
    pub color_pipeline: wgpu::RenderPipeline,
    pub normals_pipeline: wgpu::RenderPipeline,
    pub wireframe_pipeline: wgpu::RenderPipeline,
    pub depth_viz_pipeline: wgpu::RenderPipeline,
    pub depth_viz_layout: wgpu::BindGroupLayout,
    pub depth_sampler: wgpu::Sampler,
}

impl RenderResources {
    pub fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        vertex_pool: &wgpu::Buffer,
        material_table_layout: &wgpu::BindGroupLayout,
        _material_table_bind_group: &wgpu::BindGroup,
        gi_layout: &wgpu::BindGroupLayout,
        solid_shader_source: &str,
    ) -> Self {
        // Camera uniform buffer (80 bytes: mat4x4f + vec4f)
        let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera-uniform"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Group 0: Camera uniform
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera-bg"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buf.as_entire_binding(),
            }],
        });

        // Group 1: Vertex pool (storage, read-only)
        let vertex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vertex-read-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let vertex_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vertex-read-bg"),
            layout: &vertex_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: vertex_pool.as_entire_binding(),
            }],
        });

        // ── Shaders ──

        // Solid shader source is provided by the active GI backend.
        // Each backend returns a complete WGSL string (with any needed
        // prepends already applied) via GiBackend::consumer_shader_source().
        let solid_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("solid-shader"),
            source: wgpu::ShaderSource::Wgsl(solid_shader_source.into()),
        });

        let depth_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("depth-prepass-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/depth_prepass.wgsl").into()),
        });

        let normals_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("normals-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/normals.wgsl").into()),
        });

        let wireframe_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("wireframe-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/wireframe.wgsl").into()),
        });

        // ── Pipeline layouts ──

        // Depth prepass: [camera, vertex_pool] — no material needed
        let depth_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("depth-layout"),
            bind_group_layouts: &[Some(&camera_layout), Some(&vertex_layout)],
            immediate_size: 0,
        });

        // Color/normals/wireframe: [camera, vertex_pool, material_table, gi]
        // Group 3 layout is provided by the active GI backend.
        let color_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("color-layout"),
            bind_group_layouts: &[
                Some(&camera_layout),
                Some(&vertex_layout),
                Some(material_table_layout),
                Some(gi_layout),
            ],
            immediate_size: 0,
        });

        // ── Shared state ──

        let prim = wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        };

        let depth_write = wgpu::DepthStencilState {
            format: depth_format,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };

        let depth_readonly = wgpu::DepthStencilState {
            format: depth_format,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };

        let color_target = Some(wgpu::ColorTargetState {
            format: color_format,
            blend: Some(wgpu::BlendState::REPLACE),
            write_mask: wgpu::ColorWrites::ALL,
        });

        // ── R-2 Depth prepass pipeline (writes depth + normal G-buffer) ──

        let depth_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("depth-prepass-pipeline"),
            layout: Some(&depth_layout),
            vertex: wgpu::VertexState {
                module: &depth_shader,
                entry_point: Some("vs_depth"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &depth_shader,
                entry_point: Some("fs_depth"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: prim,
            depth_stencil: Some(depth_write.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // ── R-5 Color pipeline (depth read-only) ──

        let color_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("color-pipeline"),
            layout: Some(&color_layout),
            vertex: wgpu::VertexState {
                module: &solid_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &solid_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[color_target.clone()],
            }),
            primitive: prim,
            depth_stencil: Some(depth_readonly.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // ── Normals pipeline (depth read-only) ──

        let normals_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("normals-pipeline"),
            layout: Some(&color_layout),
            vertex: wgpu::VertexState {
                module: &normals_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &normals_shader,
                entry_point: Some("fs_normals"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[color_target.clone()],
            }),
            primitive: prim,
            depth_stencil: Some(depth_readonly.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // ── Wireframe pipeline (LineList topology, depth read-only) ──

        // Wireframe uses its own layout: [camera, vertex_pool] (no materials needed)
        let wire_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wireframe-layout"),
            bind_group_layouts: &[Some(&camera_layout), Some(&vertex_layout)],
            immediate_size: 0,
        });

        let wire_prim = wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::LineList,
            ..Default::default()
        };

        let wireframe_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("wireframe-pipeline"),
            layout: Some(&wire_layout),
            vertex: wgpu::VertexState {
                module: &wireframe_shader,
                entry_point: Some("vs_wire"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &wireframe_shader,
                entry_point: Some("fs_wire"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[color_target],
            }),
            primitive: wire_prim,
            depth_stencil: Some(depth_write.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // ── Depth viz pipeline (fullscreen, reads depth texture) ──

        let depth_viz_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("depth-viz-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/depth_viz.wgsl").into()),
        });

        let depth_viz_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("depth-viz-tex-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
        });

        let depth_viz_pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("depth-viz-pipe-layout"),
            bind_group_layouts: &[Some(&camera_layout), Some(&depth_viz_layout)],
            immediate_size: 0,
        });

        let depth_viz_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("depth-viz-pipeline"),
            layout: Some(&depth_viz_pipe_layout),
            vertex: wgpu::VertexState {
                module: &depth_viz_shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &depth_viz_shader,
                entry_point: Some("fs_depth_viz"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let depth_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("depth-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Cascade debug viz pipelines moved to V2LegacyBackend::create_debug_viz().

        Self {
            camera_buf,
            camera_layout,
            camera_bind_group,
            vertex_layout,
            vertex_bind_group,
            depth_pipeline,
            color_pipeline,
            normals_pipeline,
            wireframe_pipeline,
            depth_viz_pipeline,
            depth_viz_layout,
            depth_sampler,
        }
    }

    /// Create a bind group for depth viz that references the current depth texture view.
    pub fn create_depth_viz_bind_group(
        &self,
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("depth-viz-bg"),
            layout: &self.depth_viz_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.depth_sampler),
                },
            ],
        })
    }

    /// Update the camera uniform buffer.
    pub fn update_camera(
        &self,
        queue: &wgpu::Queue,
        view_proj: &glam::Mat4,
        position: &glam::Vec3,
    ) {
        let uniform = CameraUniform {
            view_proj: view_proj.to_cols_array(),
            position: [position.x, position.y, position.z, 0.0],
        };
        queue.write_buffer(&self.camera_buf, 0, bytemuck::bytes_of(&uniform));
    }
}
