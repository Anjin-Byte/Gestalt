//! Frustum Pre-Cull: lightweight AABB vs frustum test before depth prepass.
//!
//! Patches the indirect buffer to zero instance_count for off-screen chunks,
//! preventing them from polluting the Hi-Z depth buffer with mispositioned depth.

use wgpu::util::DeviceExt;

pub struct FrustumCullPass {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CullParams {
    slot_count: u32,
    _pad: [u32; 3],
}

impl FrustumCullPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("frustum-cull-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/frustum_cull.wgsl").into(),
            ),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frustum-cull-layout"),
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
                // @binding(3) indirect_buf (read-write)
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
                // @binding(4) cull_params uniform
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
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("frustum-cull-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("frustum-cull-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self { pipeline, layout }
    }

    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        camera_buf: &wgpu::Buffer,
        aabb_buf: &wgpu::Buffer,
        flags_buf: &wgpu::Buffer,
        indirect_buf: &wgpu::Buffer,
        slot_count: u32,
    ) -> wgpu::BindGroup {
        let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("frustum-cull-params"),
            contents: bytemuck::cast_slice(&[CullParams {
                slot_count,
                _pad: [0; 3],
            }]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frustum-cull-bg"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: camera_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: aabb_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: flags_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: indirect_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: params_buf.as_entire_binding() },
            ],
        })
    }

    pub fn dispatch(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bind_group: &wgpu::BindGroup,
        slot_count: u32,
    ) {
        if slot_count == 0 { return; }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("frustum-pre-cull"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, Some(bind_group), &[]);
        pass.dispatch_workgroups((slot_count + 63) / 64, 1, 1);
    }
}
