//! R-3: Hi-Z Pyramid Build Pass
//!
//! Builds a max-depth mip pyramid from the depth buffer every frame.
//! Mip 0 copies from depth_texture. Mips 1..N downsample via max of 2×2.
//!
//! See: docs/Resident Representation/stages/R-3-hiz-build.md

/// Uniform params passed to Hi-Z shaders.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct HizParams {
    src_width: u32,
    src_height: u32,
    dst_width: u32,
    dst_height: u32,
}

/// GPU compute pass for Hi-Z pyramid generation.
pub struct HizBuildPass {
    mip0_pipeline: wgpu::ComputePipeline,
    mip0_layout: wgpu::BindGroupLayout,
    downsample_pipeline: wgpu::ComputePipeline,
    downsample_layout: wgpu::BindGroupLayout,
}

impl HizBuildPass {
    /// Create pipelines (call once at init). Bind groups are created per-resize
    /// because they reference textures that change size.
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hiz-build-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/hiz_build.wgsl").into(),
            ),
        });

        // ── Mip 0 layout: group(0) ──
        let mip0_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hiz-mip0-layout"),
            entries: &[
                // @binding(0) depth_src: texture_depth_2d
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // @binding(1) hiz_dst: texture_storage_2d<r32float, write>
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::R32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                // @binding(2) params: uniform
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
            ],
        });

        let mip0_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hiz-mip0-pipeline-layout"),
            bind_group_layouts: &[Some(&mip0_layout)],
            immediate_size: 0,
        });

        let mip0_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("hiz-mip0-pipeline"),
            layout: Some(&mip0_pipeline_layout),
            module: &shader,
            entry_point: Some("build_mip0"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // ── Downsample layout: group(1) ──
        let downsample_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("hiz-downsample-layout"),
                entries: &[
                    // @binding(0) hiz_src: texture_2d<f32>
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // @binding(1) hiz_ds_dst: texture_storage_2d<r32float, write>
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: wgpu::TextureFormat::R32Float,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    // @binding(2) ds_params: uniform
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
                ],
            });

        let ds_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hiz-downsample-pipeline-layout"),
            // The downsample shader uses group(1), but pipeline layouts are ordered
            // by set index. We need group(0) empty and group(1) for downsample.
            // Simpler: use group(0) for downsample too — separate pipeline layout.
            bind_group_layouts: &[Some(&downsample_layout)],
            immediate_size: 0,
        });

        let downsample_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("hiz-downsample-pipeline"),
                layout: Some(&ds_pipeline_layout),
                module: &shader,
                entry_point: Some("build_mip"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });

        Self {
            mip0_pipeline,
            mip0_layout,
            downsample_pipeline,
            downsample_layout,
        }
    }

    /// Build bind groups for the current depth + hiz textures. Call on init and resize.
    pub fn create_bind_groups(
        &self,
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
        hiz_mip_views: &[wgpu::TextureView],
        width: u32,
        height: u32,
    ) -> HizBindGroups {
        let mip_count = hiz_mip_views.len() as u32;

        // Mip 0 bind group: depth → hiz[0]
        let mip0_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("hiz-mip0-params"),
            contents: bytemuck::cast_slice(&[HizParams {
                src_width: width,
                src_height: height,
                dst_width: width,
                dst_height: height,
            }]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let mip0_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hiz-mip0-bg"),
            layout: &self.mip0_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&hiz_mip_views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: mip0_params.as_entire_binding(),
                },
            ],
        });

        // Downsample bind groups: hiz[i] → hiz[i+1] for i in 0..mip_count-1
        let mut ds_bind_groups = Vec::new();
        let mut ds_dispatches = Vec::new();
        let mut w = width;
        let mut h = height;

        for i in 0..(mip_count - 1) {
            let src_w = w;
            let src_h = h;
            w = (w / 2).max(1);
            h = (h / 2).max(1);

            let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("hiz-ds-params-{}", i + 1)),
                contents: bytemuck::cast_slice(&[HizParams {
                    src_width: src_w,
                    src_height: src_h,
                    dst_width: w,
                    dst_height: h,
                }]),
                usage: wgpu::BufferUsages::UNIFORM,
            });

            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("hiz-ds-bg-{}", i + 1)),
                layout: &self.downsample_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&hiz_mip_views[i as usize]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(
                            &hiz_mip_views[(i + 1) as usize],
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: params_buf.as_entire_binding(),
                    },
                ],
            });

            ds_bind_groups.push(bg);
            ds_dispatches.push((w, h));
        }

        HizBindGroups {
            mip0_bg,
            mip0_dispatch: (width, height),
            ds_bind_groups,
            ds_dispatches,
        }
    }

    /// Dispatch all mip levels into the command encoder.
    pub fn dispatch(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bind_groups: &HizBindGroups,
    ) {
        // Mip 0: depth → hiz[0]
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("R-3-hiz-mip0"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.mip0_pipeline);
            pass.set_bind_group(0, Some(&bind_groups.mip0_bg), &[]);
            let (w, h) = bind_groups.mip0_dispatch;
            pass.dispatch_workgroups((w + 7) / 8, (h + 7) / 8, 1);
        }

        // Mip 1..N: downsample
        for (i, bg) in bind_groups.ds_bind_groups.iter().enumerate() {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(&format!("R-3-hiz-mip{}", i + 1)),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.downsample_pipeline);
            pass.set_bind_group(0, Some(bg), &[]);
            let (w, h) = bind_groups.ds_dispatches[i];
            pass.dispatch_workgroups((w + 7) / 8, (h + 7) / 8, 1);
        }
    }
}

/// Pre-built bind groups for one frame's Hi-Z dispatch.
/// Recreated on resize (texture dimensions change).
pub struct HizBindGroups {
    mip0_bg: wgpu::BindGroup,
    mip0_dispatch: (u32, u32),
    ds_bind_groups: Vec<wgpu::BindGroup>,
    ds_dispatches: Vec<(u32, u32)>,
}

/// Re-export wgpu::util for buffer_init
use wgpu::util::DeviceExt;
