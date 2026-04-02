//! I-3: Summary Rebuild Compute Pass
//!
//! Reads occupancy atlas, writes derived summaries (bricklet grid, flags, AABB).
//! One workgroup per chunk slot, 256 threads per workgroup.
//!
//! See: docs/Resident Representation/stages/I-3-summary-rebuild.md

/// GPU compute pipeline for I-3 summary rebuild.
pub struct SummaryPass {
    pipeline: wgpu::ComputePipeline,
}

impl SummaryPass {
    /// Create the compute pipeline. Call once during Renderer init.
    pub fn new(
        device: &wgpu::Device,
        bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("summary-rebuild-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/summary_rebuild.wgsl").into(),
            ),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("summary-rebuild-layout"),
            bind_group_layouts: &[Some(bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("summary-rebuild-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self { pipeline }
    }

    /// Record I-3 dispatch into a command encoder.
    /// `slot_count` = number of chunk slots to rebuild (one workgroup each).
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
            label: Some("I-3-summary-rebuild"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, Some(bind_group), &[]);
        pass.dispatch_workgroups(slot_count, 1, 1);
    }
}
