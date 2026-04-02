//! R-1: Mesh Rebuild Compute Pass
//!
//! Reads occupancy atlas + palette, writes vertex pool + index pool + draw metadata.
//! Dispatch: (slot_count, 6, 1) — one workgroup per face direction per chunk.
//! @workgroup_size(64, 1, 1) — one thread per slice (62 active, 2 idle).
//!
//! See: docs/Resident Representation/stages/R-1-mesh-rebuild.md

use crate::pool::*;

/// GPU compute pipeline for R-1 mesh rebuild.
pub struct MeshPass {
    pipeline: wgpu::ComputePipeline,
}

impl MeshPass {
    /// Create the compute pipeline. Call once during Renderer init.
    pub fn new(
        device: &wgpu::Device,
        bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh-rebuild-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/mesh_rebuild.wgsl").into(),
            ),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh-rebuild-layout"),
            bind_group_layouts: &[Some(bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("mesh-rebuild-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self { pipeline }
    }

    /// Zero draw metadata for `slot_count` slots before dispatch.
    pub fn clear_draw_meta(queue: &wgpu::Queue, pool: &crate::pool_gpu::ChunkPool, slot_count: u32) {
        let zeros = vec![0u8; DRAW_META_BYTES as usize * slot_count as usize];
        queue.write_buffer(pool.draw_meta_buf(), 0, &zeros);
    }

    /// Record R-1 dispatch into a command encoder.
    /// `slot_count` = number of chunk slots to rebuild.
    /// Dispatches (slot_count, 6, 1) — 6 workgroups per slot (one per face direction).
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
            label: Some("R-1-mesh-rebuild"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, Some(bind_group), &[]);
        pass.dispatch_workgroups(slot_count, 6, 1);
    }
}
