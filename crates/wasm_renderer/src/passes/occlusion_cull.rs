//! R-4 Phase 1: Chunk-Level Occlusion Cull
//!
//! Tests chunk AABBs against frustum + Hi-Z pyramid.
//! Writes per-slot visibility (0=culled, 1=visible) for indirect draw filtering.
//!
//! See: docs/Resident Representation/stages/R-4-occlusion-cull.md

use wgpu::util::DeviceExt;

/// GPU compute pass for chunk-level occlusion culling.
pub struct OcclusionCullPass {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

/// Uniform params for the cull shader.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CullParams {
    slot_count: u32,
    hiz_width: u32,
    hiz_height: u32,
    _pad: u32,
}

impl OcclusionCullPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("occlusion-cull-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/occlusion_cull.wgsl").into(),
            ),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("occlusion-cull-layout"),
            entries: &[
                // @binding(0) camera uniform
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // @binding(1) aabb_buf (read)
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
                // @binding(2) flags_buf (read)
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
                // @binding(3) hiz_pyramid texture
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
                // @binding(4) visibility (read-write)
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
                // @binding(5) cull_params uniform
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
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
            label: Some("occlusion-cull-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("occlusion-cull-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self { pipeline, layout }
    }

    /// Create a bind group for this frame. Call each frame (or on resize) since
    /// the Hi-Z texture view changes.
    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        camera_buf: &wgpu::Buffer,
        aabb_buf: &wgpu::Buffer,
        flags_buf: &wgpu::Buffer,
        hiz_view: &wgpu::TextureView,
        visibility_buf: &wgpu::Buffer,
        slot_count: u32,
        hiz_width: u32,
        hiz_height: u32,
    ) -> wgpu::BindGroup {
        let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cull-params"),
            contents: bytemuck::cast_slice(&[CullParams {
                slot_count,
                hiz_width,
                hiz_height,
                _pad: 0,
            }]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("occlusion-cull-bg"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: aabb_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: flags_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(hiz_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: visibility_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: params_buf.as_entire_binding(),
                },
            ],
        })
    }

    /// Dispatch the cull pass.
    pub fn dispatch(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bind_group: &wgpu::BindGroup,
        slot_count: u32,
    ) {
        if slot_count == 0 {
            return;
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("R-4-occlusion-cull"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, Some(bind_group), &[]);
        pass.dispatch_workgroups((slot_count + 63) / 64, 1, 1);
    }
}
