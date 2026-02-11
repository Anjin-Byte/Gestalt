//! Compute pipeline creation for the voxelizer.

use std::collections::HashMap;

use super::shaders::{COMPACT_ATTRS_WGSL, COMPACT_WGSL, VOXELIZER_WGSL};

/// Collection of compute pipelines used by the voxelizer.
pub struct GpuPipelines {
    pub pipeline: wgpu::ComputePipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub compact_pipeline: wgpu::ComputePipeline,
    pub compact_bind_group_layout: wgpu::BindGroupLayout,
    pub compact_attrs_pipeline: wgpu::ComputePipeline,
    pub compact_attrs_bind_group_layout: wgpu::BindGroupLayout,
}

/// Creates all compute pipelines for the voxelizer.
pub async fn create_pipelines(
    device: &wgpu::Device,
    workgroup_size: u32,
    tiles_per_workgroup: u32,
) -> Result<GpuPipelines, String> {
    let voxelizer = create_voxelizer_pipeline(device, workgroup_size, tiles_per_workgroup);
    let compact = create_compact_pipeline(device).await?;
    let compact_attrs = create_compact_attrs_pipeline(device);

    Ok(GpuPipelines {
        pipeline: voxelizer.0,
        bind_group_layout: voxelizer.1,
        compact_pipeline: compact.0,
        compact_bind_group_layout: compact.1,
        compact_attrs_pipeline: compact_attrs.0,
        compact_attrs_bind_group_layout: compact_attrs.1,
    })
}

// === Voxelizer Pipeline ===

fn create_voxelizer_pipeline(
    device: &wgpu::Device,
    workgroup_size: u32,
    tiles_per_workgroup: u32,
) -> (wgpu::ComputePipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("voxelizer.wgsl"),
        source: wgpu::ShaderSource::Wgsl(VOXELIZER_WGSL.into()),
    });

    let bind_group_layout = create_voxelizer_bind_group_layout(device);

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("voxelizer.pipeline_layout"),
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    let mut constants = HashMap::new();
    constants.insert("WORKGROUP_SIZE".to_string(), workgroup_size as f64);
    constants.insert("TILES_PER_WORKGROUP".to_string(), tiles_per_workgroup as f64);

    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("voxelizer.pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: "main",
        compilation_options: wgpu::PipelineCompilationOptions {
            constants: &constants,
            ..Default::default()
        },
        cache: None,
    });

    (pipeline, bind_group_layout)
}

fn create_voxelizer_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("voxelizer.bind_group_layout"),
        entries: &[
            storage_buffer_entry(0, true),  // tris
            storage_buffer_entry(3, true),  // tile_offsets
            storage_buffer_entry(4, true),  // tri_indices
            storage_buffer_entry(6, false), // occupancy
            storage_buffer_entry(7, false), // owner_id
            storage_buffer_entry(8, false), // color_rgba
            uniform_buffer_entry(9),        // params
            storage_buffer_entry(10, true), // brick_origins
            storage_buffer_entry(11, false), // debug_counts
        ],
    })
}

// === Compact Pipeline ===

async fn create_compact_pipeline(
    device: &wgpu::Device,
) -> Result<(wgpu::ComputePipeline, wgpu::BindGroupLayout), String> {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("voxelizer.compact.wgsl"),
        source: wgpu::ShaderSource::Wgsl(COMPACT_WGSL.into()),
    });

    let bind_group_layout = create_compact_bind_group_layout(device);

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("voxelizer.compact_pipeline_layout"),
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    device.push_error_scope(wgpu::ErrorFilter::Validation);

    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("voxelizer.compact_pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: "main",
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });

    if let Some(err) = device.pop_error_scope().await {
        return Err(format!("Compact pipeline validation error: {err}"));
    }

    Ok((pipeline, bind_group_layout))
}

fn create_compact_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("voxelizer.compact_bind_group_layout"),
        entries: &[
            storage_buffer_entry(0, true),  // occupancy
            storage_buffer_entry(1, true),  // brick_origins
            storage_buffer_entry(2, false), // out_positions
            storage_buffer_entry(3, false), // counter
            uniform_buffer_entry(4),        // params
            storage_buffer_entry(5, false), // debug
        ],
    })
}

// === Compact Attrs Pipeline ===

fn create_compact_attrs_pipeline(
    device: &wgpu::Device,
) -> (wgpu::ComputePipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("voxelizer.compact_attrs.wgsl"),
        source: wgpu::ShaderSource::Wgsl(COMPACT_ATTRS_WGSL.into()),
    });

    let bind_group_layout = create_compact_attrs_bind_group_layout(device);

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("voxelizer.compact_attrs_pipeline_layout"),
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("voxelizer.compact_attrs_pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: "main",
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });

    (pipeline, bind_group_layout)
}

fn create_compact_attrs_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("voxelizer.compact_attrs_bind_group_layout"),
        entries: &[
            storage_buffer_entry(0, true),  // occupancy
            storage_buffer_entry(1, true),  // brick_origins
            storage_buffer_entry(2, true),  // owner_id
            storage_buffer_entry(3, true),  // color_rgba
            storage_buffer_entry(4, false), // out_indices
            storage_buffer_entry(5, false), // out_owner
            storage_buffer_entry(6, false), // out_color
            storage_buffer_entry(7, false), // counter
            uniform_buffer_entry(8),        // params
        ],
    })
}

// === Layout Entry Helpers ===

fn storage_buffer_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
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

fn uniform_buffer_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
