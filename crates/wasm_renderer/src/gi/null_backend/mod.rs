//! Null GI backend — no indirect lighting, no compute dispatch.
//!
//! Used when GI is disabled at the system level. The consumer shader
//! (`solid_null.wgsl`) renders with direct sun + hemisphere ambient only.
//! All lifecycle methods are no-ops. The consumer bind group is a single
//! dummy uniform buffer that's never read.

use crate::gi::{GiBackend, GiBuildParams};
use crate::pool;

pub struct NullBackend {
    consumer_layout: wgpu::BindGroupLayout,
    consumer_bg: wgpu::BindGroup,
    _dummy_buf: wgpu::Buffer,
}

impl NullBackend {
    pub fn new(device: &wgpu::Device) -> Self {
        // Dummy uniform buffer (16 bytes = vec4f) so the bind group is valid.
        let dummy_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gi-null-dummy"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });

        let consumer_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gi-null-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let consumer_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gi-null-bg"),
            layout: &consumer_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: dummy_buf.as_entire_binding(),
            }],
        });

        Self {
            consumer_layout,
            consumer_bg,
            _dummy_buf: dummy_buf,
        }
    }
}

impl GiBackend for NullBackend {
    fn on_chunk_resident(&mut self, _q: &wgpu::Queue, _s: u32, _c: pool::ChunkCoord) {}
    fn on_chunk_evicted(&mut self, _q: &wgpu::Queue, _s: u32, _c: pool::ChunkCoord) {}
    fn on_scene_reset(&mut self, _q: &wgpu::Queue) {}
    fn on_residency_settled(&mut self, _q: &wgpu::Queue, _a: &pool::SlotAllocator) {}

    fn dispatch_build(
        &mut self,
        _encoder: &mut wgpu::CommandEncoder,
        _queue: &wgpu::Queue,
        _params: &GiBuildParams,
    ) {
        // No-op: no compute work.
    }

    fn consumer_bind_group(&self) -> &wgpu::BindGroup {
        &self.consumer_bg
    }

    fn consumer_layout(&self) -> &wgpu::BindGroupLayout {
        &self.consumer_layout
    }

    fn consumer_shader_source(&self) -> String {
        include_str!("../../shaders/solid_null.wgsl").to_string()
    }
}
