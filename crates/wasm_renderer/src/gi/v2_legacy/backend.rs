//! v2 legacy GI backend — implements `GiBackend`.
//!
//! Screen-space probes, ping-pong cascade atlases, bilateral merge.
//! Absorbs all v2-specific GPU state that previously lived in `Renderer`
//! and `gpu::RenderResources`.

use crate::gi::{GiBackend, GiBuildParams, GiDebugParams};
use crate::pool;

// ─── v2 constants (moved from gpu.rs) ──────────────────────────────────

pub const N_CASCADES: u32 = 6;
pub const CASCADE_UNIFORM_ALIGNED_SIZE: u64 = 256;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CascadeUniforms {
    pub proj_inv: [f32; 16],
    pub view_inv: [f32; 16],
    pub screen_size: [u32; 2],
    pub cascade_index: u32,
    pub frame_index: u32,
    pub voxel_scale: f32,
    pub _pad1: [f32; 3],
    pub grid_origin: [f32; 3],
    pub bounce_intensity: f32,
    pub prev_view_proj: [f32; 16],
}

fn create_cascade_atlas(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    label: &str,
    extra_usage: wgpu::TextureUsages,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | extra_usage,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

// ─── Backend struct ────────────────────────────────────────────────────

pub struct V2LegacyBackend {
    build_pass: super::CascadeBuildPass,
    atlas_a: (wgpu::Texture, wgpu::TextureView),
    atlas_b: (wgpu::Texture, wgpu::TextureView),
    cascade_prev: (wgpu::Texture, wgpu::TextureView),
    uniforms_buf: wgpu::Buffer,
    world_data_bg: wgpu::BindGroup,
    bg_a_to_b: wgpu::BindGroup,
    bg_b_to_a: wgpu::BindGroup,
    // Consumer (group 3 for solid_v2.wgsl)
    consumer_layout: wgpu::BindGroupLayout,
    consumer_bg: wgpu::BindGroup,
    // Debug viz
    debug_pipelines: Vec<wgpu::RenderPipeline>,
    debug_layout: wgpu::BindGroupLayout,
    // Screen dimensions (needed for dispatch + resize)
    width: u32,
    height: u32,
}

impl V2LegacyBackend {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        width: u32,
        height: u32,
        color_format: wgpu::TextureFormat,
        depth_view: &wgpu::TextureView,
        normal_view: &wgpu::TextureView,
        occupancy_buf: &wgpu::Buffer,
        flags_buf: &wgpu::Buffer,
        slot_table_buf: &wgpu::Buffer,
        material_table_buf: &wgpu::Buffer,
        palette_buf: &wgpu::Buffer,
        palette_meta_buf: &wgpu::Buffer,
        index_buf_pool_buf: &wgpu::Buffer,
        slot_table_params_buf: &wgpu::Buffer,
    ) -> Self {
        let build_pass = super::CascadeBuildPass::new(device);

        // Cascade uniform buffer: N_CASCADES × 256-byte aligned slots
        let uniforms_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("v2-cascade-uniforms"),
            size: N_CASCADES as u64 * CASCADE_UNIFORM_ALIGNED_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Cascade atlases (ping-pong A/B + prev for multi-bounce)
        let atlas_a = create_cascade_atlas(device, width, height, "v2-cascade-atlas-a", wgpu::TextureUsages::COPY_SRC);
        let atlas_b = create_cascade_atlas(device, width, height, "v2-cascade-atlas-b", wgpu::TextureUsages::empty());
        let cascade_prev = create_cascade_atlas(device, width, height, "v2-cascade-prev", wgpu::TextureUsages::COPY_DST);

        // World data bind group (group 0)
        let world_data_bg = build_pass.create_world_data_bind_group(
            device,
            occupancy_buf, flags_buf, slot_table_buf,
            material_table_buf, palette_buf, palette_meta_buf,
            index_buf_pool_buf, slot_table_params_buf,
        );

        // Ping-pong bind groups (group 1)
        let bg_a_to_b = build_pass.create_per_pass_bind_group(
            device, &uniforms_buf, depth_view,
            &atlas_b.1, &atlas_a.1, normal_view, &cascade_prev.1,
        );
        let bg_b_to_a = build_pass.create_per_pass_bind_group(
            device, &uniforms_buf, depth_view,
            &atlas_a.1, &atlas_b.1, normal_view, &cascade_prev.1,
        );

        // Consumer layout: single texture binding (the cascade atlas)
        let consumer_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("v2-cascade-gi-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });

        let consumer_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("v2-cascade-gi-bg"),
            layout: &consumer_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&atlas_a.1),
            }],
        });

        // Debug viz pipelines
        let (debug_layout, debug_pipelines) =
            Self::create_debug_viz(device, color_format, &uniforms_buf);

        Self {
            build_pass,
            atlas_a,
            atlas_b,
            cascade_prev,
            uniforms_buf,
            world_data_bg,
            bg_a_to_b,
            bg_b_to_a,
            consumer_layout,
            consumer_bg,
            debug_pipelines,
            debug_layout,
            width,
            height,
        }
    }

    fn create_debug_viz(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        _uniforms_buf: &wgpu::Buffer,
    ) -> (wgpu::BindGroupLayout, Vec<wgpu::RenderPipeline>) {
        let debug_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("v2-cascade-debug-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("shaders/cascade_debug.wgsl").into(),
            ),
        });

        let debug_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("v2-cascade-debug-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("v2-cascade-debug-pipe-layout"),
            bind_group_layouts: &[Some(&debug_layout)],
            immediate_size: 0,
        });

        let entry_points = [
            "fs_cascade_raw", "fs_opacity_map", "fs_gi_only", "fs_atlas_b",
            "fs_depth_normals", "fs_world_pos", "fs_raw_texel", "fs_single_dir",
        ];
        let pipelines: Vec<_> = entry_points.iter().map(|ep| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&format!("v2-cascade-debug-{ep}")),
                layout: Some(&pipe_layout),
                vertex: wgpu::VertexState {
                    module: &debug_shader,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &debug_shader,
                    entry_point: Some(ep),
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
            })
        }).collect();

        (debug_layout, pipelines)
    }
}

impl GiBackend for V2LegacyBackend {
    fn on_chunk_resident(&mut self, _queue: &wgpu::Queue, _chunk_slot: u32, _coord: pool::ChunkCoord) {
        // v2 is screen-space; no per-chunk probe data.
    }

    fn on_chunk_evicted(&mut self, _queue: &wgpu::Queue, _chunk_slot: u32, _coord: pool::ChunkCoord) {
        // No-op.
    }

    fn on_scene_reset(&mut self, _queue: &wgpu::Queue) {
        // No-op.
    }

    fn on_residency_settled(&mut self, _queue: &wgpu::Queue, _allocator: &pool::SlotAllocator) {
        // No-op.
    }

    fn dispatch_build(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        params: &GiBuildParams,
    ) {
        let sw = params.screen_width;
        let sh = params.screen_height;

        if params.gi_enabled && params.resident_count > 0 {
            // Pre-write all cascade uniform slots
            for ci in 0..N_CASCADES {
                let offset = ci as u64 * CASCADE_UNIFORM_ALIGNED_SIZE;
                let uniforms = CascadeUniforms {
                    proj_inv: params.camera_proj_inv.to_cols_array(),
                    view_inv: params.camera_view_inv.to_cols_array(),
                    screen_size: [sw, sh],
                    cascade_index: ci,
                    frame_index: params.frame_index,
                    voxel_scale: params.voxel_scale,
                    _pad1: [0.0; 3],
                    grid_origin: params.grid_origin,
                    bounce_intensity: 0.8,
                    prev_view_proj: params.prev_view_proj.to_cols_array(),
                };
                queue.write_buffer(&self.uniforms_buf, offset, bytemuck::bytes_of(&uniforms));
            }

            // Ping-pong dispatch: cascade N-1 → 0
            for ci in (0..N_CASCADES).rev() {
                let dyn_offset = ci * CASCADE_UNIFORM_ALIGNED_SIZE as u32;
                let bg = if (N_CASCADES - 1 - ci) % 2 == 0 {
                    &self.bg_a_to_b
                } else {
                    &self.bg_b_to_a
                };
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some(&format!("v2-cascade-build-{ci}")),
                    timestamp_writes: None,
                });
                pass.set_pipeline(self.build_pass.pipeline());
                pass.set_bind_group(0, Some(&self.world_data_bg), &[]);
                pass.set_bind_group(1, Some(bg), &[dyn_offset]);
                pass.dispatch_workgroups((sw + 7) / 8, (sh + 7) / 8, 1);
            }

            // Copy cascade 0 (atlas A) → cascade_prev for multi-bounce
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas_a.0,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.cascade_prev.0,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d { width: sw, height: sh, depth_or_array_layers: 1 },
            );
        } else {
            // GI disabled: clear atlas A with sentinel
            let uniforms = CascadeUniforms {
                proj_inv: params.camera_proj_inv.to_cols_array(),
                view_inv: params.camera_view_inv.to_cols_array(),
                screen_size: [sw, sh],
                cascade_index: 0xFFFFFFFF,
                frame_index: params.frame_index,
                voxel_scale: params.voxel_scale,
                _pad1: [0.0; 3],
                grid_origin: params.grid_origin,
                bounce_intensity: 0.0,
                prev_view_proj: params.prev_view_proj.to_cols_array(),
            };
            queue.write_buffer(&self.uniforms_buf, 0, bytemuck::bytes_of(&uniforms));
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("v2-cascade-clear"),
                timestamp_writes: None,
            });
            pass.set_pipeline(self.build_pass.pipeline());
            pass.set_bind_group(0, Some(&self.world_data_bg), &[]);
            pass.set_bind_group(1, Some(&self.bg_b_to_a), &[0]);
            pass.dispatch_workgroups((sw + 7) / 8, (sh + 7) / 8, 1);
        }
    }

    fn on_resize(
        &mut self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        width: u32,
        height: u32,
        depth_view: &wgpu::TextureView,
        normal_view: &wgpu::TextureView,
    ) {
        self.width = width;
        self.height = height;

        self.atlas_a = create_cascade_atlas(device, width, height, "v2-cascade-atlas-a", wgpu::TextureUsages::COPY_SRC);
        self.atlas_b = create_cascade_atlas(device, width, height, "v2-cascade-atlas-b", wgpu::TextureUsages::empty());
        self.cascade_prev = create_cascade_atlas(device, width, height, "v2-cascade-prev", wgpu::TextureUsages::COPY_DST);

        self.bg_a_to_b = self.build_pass.create_per_pass_bind_group(
            device, &self.uniforms_buf, depth_view,
            &self.atlas_b.1, &self.atlas_a.1, normal_view, &self.cascade_prev.1,
        );
        self.bg_b_to_a = self.build_pass.create_per_pass_bind_group(
            device, &self.uniforms_buf, depth_view,
            &self.atlas_a.1, &self.atlas_b.1, normal_view, &self.cascade_prev.1,
        );

        self.consumer_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("v2-cascade-gi-bg"),
            layout: &self.consumer_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&self.atlas_a.1),
            }],
        });
    }

    fn consumer_bind_group(&self) -> &wgpu::BindGroup {
        &self.consumer_bg
    }

    fn consumer_layout(&self) -> &wgpu::BindGroupLayout {
        &self.consumer_layout
    }

    fn consumer_shader_source(&self) -> String {
        include_str!("../../shaders/solid_v2.wgsl").to_string()
    }

    fn debug_render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        device: &wgpu::Device,
        params: &GiDebugParams,
    ) -> bool {
        // Upload debug camera uniforms (proj_inv + view_inv = 128 bytes)
        let debug_cam_data: [f32; 32] = {
            let mut d = [0.0f32; 32];
            d[..16].copy_from_slice(&params.camera_proj_inv.to_cols_array());
            d[16..32].copy_from_slice(&params.camera_view_inv.to_cols_array());
            d
        };
        queue.write_buffer(&self.uniforms_buf, 0, bytemuck::cast_slice(&debug_cam_data));

        let debug_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("v2-cascade-debug-bg"),
            layout: &self.debug_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.atlas_a.1) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.atlas_b.1) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(params.depth_view) },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.uniforms_buf,
                        offset: 0,
                        size: Some(std::num::NonZeroU64::new(128).unwrap()),
                    }),
                },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(params.normal_view) },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("v2-cascade-debug"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: params.target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        let pipeline_idx = params.mode as usize;
        if pipeline_idx < self.debug_pipelines.len() {
            pass.set_pipeline(&self.debug_pipelines[pipeline_idx]);
            pass.set_bind_group(0, Some(&debug_bg), &[]);
            pass.draw(0..3, 0..1);
        }

        true
    }
}
