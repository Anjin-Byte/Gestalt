//! Build Wireframe Indices — converts quad triangle indices to edge line indices.

/// GPU compute pipeline for building wireframe edge indices.
pub struct BuildWireframePass {
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
}

impl BuildWireframePass {
    pub fn new(
        device: &wgpu::Device,
        index_pool: &wgpu::Buffer,
        mesh_offset_table: &wgpu::Buffer,
        wire_index_pool: &wgpu::Buffer,
        wire_indirect_buf: &wgpu::Buffer,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("build-wireframe-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/build_wireframe.wgsl").into(),
            ),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("build-wireframe-layout"),
            entries: &[
                storage_entry(0, true),  // index_pool (read)
                storage_entry(1, true),  // mesh_offset_table (read)
                storage_entry(2, false), // wire_index_pool (write)
                storage_entry(3, false), // wire_indirect (write)
            ],
        });

        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("build-wireframe-pipe-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("build-wireframe-pipeline"),
            layout: Some(&pipe_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("build-wireframe-bg"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: index_pool.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: mesh_offset_table.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wire_index_pool.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wire_indirect_buf.as_entire_binding() },
            ],
        });

        Self { pipeline, bind_group }
    }

    /// Dispatch one workgroup per slot. All 64 threads in the workgroup cooperate
    /// on the slot's quads via strided distribution (F3 parallelization).
    pub fn dispatch(&self, encoder: &mut wgpu::CommandEncoder, slot_count: u32) {
        if slot_count == 0 { return; }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("build-wireframe"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, Some(&self.bind_group), &[]);
        pass.dispatch_workgroups(slot_count, 1, 1);
    }
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
