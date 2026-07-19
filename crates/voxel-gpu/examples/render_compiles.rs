//! Headless shader + pipeline validation for the deferred renderer: construct a
//! `GpuRenderer` (which compiles EVERY pass assembly — gbuffer, skip-transparent,
//! palette, GTAO, denoise, composite, TAA, blend — so naga validates them all)
//! and render one frame over a tiny structure (the wgpu validation layer then
//! checks the multi-pass bind groups + dispatch), without opening a window. Runs
//! both the static occupancy path and the editable paged-truecolor path.
//! Run: `cargo run -p voxel-gpu --example render_compiles`

#![allow(clippy::cast_precision_loss)]

use voxel_core::fixtures::OctantFractal;
use voxel_core::{Resolution, SchoolBBuffer, SparseTree, VoxelCoord};
use voxel_gpu::{GpuCamera, OUTPUT_FORMAT};

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / l, v[1] / l, v[2] / l]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// One headless frame through the full deferred chain.
fn render_once(ctx: &voxel_gpu::GpuContext, renderer: &mut voxel_gpu::GpuRenderer, n: f32, k: u32) {
    let (w, h) = (96u32, 64u32);
    let eye = [n * 1.5, n * 1.2, -n * 0.5];
    let center = [n * 0.5; 3];
    let forward = norm([center[0] - eye[0], center[1] - eye[1], center[2] - eye[2]]);
    let right = norm(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    let camera = GpuCamera {
        eye,
        tan: 0.5,
        forward,
        aspect: w as f32 / h as f32,
        right,
        n,
        up,
        pad: 0.0,
        dims: [w, h, k, 0],
    };
    let out = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless out"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: OUTPUT_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });
    let view = out.create_view(&wgpu::TextureViewDescriptor::default());
    let mut enc = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("headless"),
        });
    renderer.render(&mut enc, &camera, &view, w, h);
    ctx.queue.submit(std::iter::once(enc.finish()));
    let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
}

fn main() {
    let Ok(ctx) = voxel_gpu::GpuContext::try_new() else {
        println!("no GPU");
        return;
    };
    let res = Resolution::new(32).unwrap();
    let n = res.voxels_per_axis() as f32;
    let k = res.internal_levels();

    // Static occupancy scene: constructing the renderer compiles every pass
    // shader; the frame exercises the None-albedo g-buffer + full chain.
    let tree = SparseTree::build(&OctantFractal::sierpinski_tetrahedron(res));
    let structure = SchoolBBuffer::from_sparse(&tree);
    let mut renderer = match voxel_gpu::GpuRenderer::new(
        &ctx,
        &structure,
        &voxel_core::MaterialTable::missing_only(),
    ) {
        Ok(r) => r,
        Err(e) => {
            println!("shaders FAILED to compile: {e:?}");
            std::process::exit(1);
        }
    };
    println!("all deferred pass shaders compiled OK");
    render_once(&ctx, &mut renderer, n, k);
    println!("multi-pass render dispatched OK (occupancy)");

    // Editable paged-truecolor scene: a small voxel cloud with a colour pool
    // installed; exercises the paged colour bindings + the truecolor g-buffer arm.
    let voxels = [
        VoxelCoord::new(1, 1, 1),
        VoxelCoord::new(2, 1, 1),
        VoxelCoord::new(9, 3, 4),
    ];
    let mut colored = SparseTree::from_voxels(res, voxels.into_iter().map(|v| (v, 0u16)));
    let count = usize::try_from(colored.occupied_voxels()).unwrap_or(voxels.len());
    colored.install_colors(std::iter::repeat_n(0xFF00_C8FFu32, count));
    let paged_structure = SchoolBBuffer::from_sparse(&colored);
    let pages = colored.color_pages().expect("colored tree has a pool");
    let mut paged_renderer = match voxel_gpu::GpuRenderer::new_paged(&ctx, &paged_structure, &pages)
    {
        Ok(r) => r,
        Err(e) => {
            println!("paged renderer FAILED: {e:?}");
            std::process::exit(1);
        }
    };
    render_once(&ctx, &mut paged_renderer, n, k);
    println!("multi-pass render dispatched OK (paged truecolor)");
}
