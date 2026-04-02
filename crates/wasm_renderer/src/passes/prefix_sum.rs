//! R-1 Pass 2: Prefix Sum — computes per-slot mesh pool offsets from quad counts.
//!
//! Single-workgroup Blelloch exclusive scan. Dispatch: (1, 1, 1).
//!
//! See: docs/Resident Representation/variable-mesh-pool.md

/// GPU compute pipeline for prefix sum (Pass 2 of three-pass mesh rebuild).
pub struct PrefixSumPass {
    pipeline: wgpu::ComputePipeline,
}

impl PrefixSumPass {
    pub fn new(
        device: &wgpu::Device,
        bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("prefix-sum-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/prefix_sum.wgsl").into(),
            ),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("prefix-sum-layout"),
            bind_group_layouts: &[Some(bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("prefix-sum-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self { pipeline }
    }

    /// Dispatch prefix sum. Always (1, 1, 1) — single workgroup handles all slots.
    pub fn dispatch(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bind_group: &wgpu::BindGroup,
    ) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("R-1-prefix-sum"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, Some(bind_group), &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
}
