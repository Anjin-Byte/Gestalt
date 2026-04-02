//! R-1 Pass 1: Mesh Count — counts quads per slot without emitting geometry.
//!
//! Same algorithm as mesh_rebuild (face cull + greedy merge + material boundaries),
//! but only increments a per-slot atomic counter. Output feeds the prefix sum pass.
//!
//! See: docs/Resident Representation/variable-mesh-pool.md

/// GPU compute pipeline for quad counting (Pass 1 of three-pass mesh rebuild).
pub struct MeshCountPass {
    pipeline: wgpu::ComputePipeline,
}

impl MeshCountPass {
    pub fn new(
        device: &wgpu::Device,
        bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh-count-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/mesh_count.wgsl").into(),
            ),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh-count-layout"),
            bind_group_layouts: &[Some(bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("mesh-count-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self { pipeline }
    }

    /// Dispatch count pass. Same workgroup layout as mesh rebuild: (slot_count, 6, 1).
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
            label: Some("R-1-mesh-count"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, Some(bind_group), &[]);
        pass.dispatch_workgroups(slot_count, 6, 1);
    }
}
