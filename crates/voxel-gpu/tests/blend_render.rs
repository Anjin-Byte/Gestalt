//! GPU deferred-**transparency** render test (Stage C — the acceptance gate).
//!
//! Renders a thin semi-transparent layer in front of an opaque layer and reads the
//! framebuffer back. Proves the transparency-aware g-buffer + forward-blend pass
//! actually composite: the opaque backdrop shows THROUGH the glass. The broken
//! (pre-correction) pipeline rendered the through-glass pixel as pure lit red with
//! the backdrop fully occluded; this test fails on that regression.
//!
//! Deferred lighting means we assert hue presence (the glass and the backdrop both
//! contribute), not exact bytes. The opaque-control case (no transparency → the skip
//! variant is never selected → the blend pass isn't dispatched) confirms the opaque
//! path is unaffected: the front opaque layer occludes the back one.
//!
//! Gated like `differential.rs`: skips with no adapter unless `VOXEL_REQUIRE_GPU=1`.

#![allow(clippy::cast_precision_loss)]

use voxel_core::fixtures::Solid;
use voxel_core::{MaterialTable, Resolution, SchoolBBuffer, SparseTree};
use voxel_gpu::{GpuCamera, GpuContext, GpuError, GpuRenderer, OUTPUT_FORMAT};

fn context_or_skip() -> Option<GpuContext> {
    match GpuContext::try_new() {
        Ok(ctx) => Some(ctx),
        Err(GpuError::NoAdapter) if std::env::var_os("VOXEL_REQUIRE_GPU").is_none() => {
            eprintln!("skip: no GPU adapter (set VOXEL_REQUIRE_GPU=1 to require one)");
            None
        }
        Err(e) => panic!("GPU unavailable: {e}"),
    }
}

/// Perspective camera looking straight down +Z at the centre of an `n³` grid — the
/// central ray pierces the front layer (z=0) then the layer behind it.
fn front_camera(r: Resolution, dim: u32) -> GpuCamera {
    let n = r.voxels_per_axis() as f32;
    let half = n * 0.5;
    GpuCamera {
        eye: [half, half, -40.0],
        tan: 1.0,
        forward: [0.0, 0.0, 1.0],
        aspect: 1.0,
        right: [1.0, 0.0, 0.0],
        n,
        up: [0.0, 1.0, 0.0],
        pad: 0.0,
        dims: [dim, dim, r.internal_levels(), 0],
    }
}

fn read_render(ctx: &GpuContext, r: &mut GpuRenderer, cam: &GpuCamera, dim: u32) -> Vec<[u8; 4]> {
    let tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("blend test output"),
        size: wgpu::Extent3d {
            width: dim,
            height: dim,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: OUTPUT_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    let rb = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("blend test readback"),
        size: u64::from(dim * dim * 4),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    r.render(&mut enc, cam, &view, dim, dim);
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &rb,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(dim * 4),
                rows_per_image: Some(dim),
            },
        },
        wgpu::Extent3d {
            width: dim,
            height: dim,
            depth_or_array_layers: 1,
        },
    );
    ctx.queue.submit(std::iter::once(enc.finish()));
    let slice = rb.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        let _ = tx.send(res);
    });
    ctx.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let px: Vec<[u8; 4]> = data
        .chunks_exact(4)
        .map(|c| [c[0], c[1], c[2], c[3]])
        .collect();
    drop(data);
    rb.unmap();
    px
}

/// Render a Solid cube whose front layer (z==0) is `front` and the rest opaque green,
/// and return the central through-glass pixel.
fn through_glass(ctx: &GpuContext, front: [u8; 4]) -> [u8; 4] {
    let r = Resolution::new(32).unwrap();
    let dim = 64u32;
    let tree = SparseTree::build(&Solid { resolution: r });
    let mut s = SchoolBBuffer::from_sparse(&tree);
    // Thin front glass (one voxel deep) so the opaque green behind shows through;
    // a thick glass slab would saturate to opaque (acc_a → 1) and hide it.
    s.assemble_leaf_color(&tree, |c| if c.z < 1 { front } else { [0, 255, 0, 255] });
    let mut rndr = GpuRenderer::new(ctx, &s, &MaterialTable::missing_only()).unwrap();
    let px = read_render(ctx, &mut rndr, &front_camera(r, dim), dim);
    px[(dim as usize / 2) * dim as usize + dim as usize / 2]
}

#[test]
fn transparency_blends_backdrop_through_glass() {
    let Some(ctx) = context_or_skip() else { return };
    assert_eq!(OUTPUT_FORMAT, wgpu::TextureFormat::Rgba8Unorm);

    // (1) Semi-transparent red glass (alpha 128) over opaque green: the green backdrop
    // must show THROUGH the glass — a genuine red-over-green blend.
    let glass = through_glass(&ctx, [255, 0, 0, 128]);
    assert!(
        glass[1] > 8,
        "opaque green backdrop did not show through the transparent glass: {glass:?} \
         (regression to the occluded pure-red [74,0,0])"
    );
    assert!(
        glass[0] > 8,
        "red glass missing — not a genuine blend: {glass:?}"
    );

    // (2) Control: the SAME geometry with an OPAQUE front layer (alpha 255) → the scene
    // has no transparency, so the skip variant is never selected and the blend pass is
    // not dispatched. The opaque red front must OCCLUDE the green (no green shows).
    let opaque = through_glass(&ctx, [255, 0, 0, 255]);
    assert!(
        opaque[0] > 8 && i32::from(opaque[1]) < i32::from(opaque[0]),
        "opaque front layer should occlude the green backdrop (red-dominant, no green): {opaque:?}"
    );
    assert!(
        opaque[1] < 8,
        "green leaked through an OPAQUE front layer ({opaque:?}) — the opaque path is wrong"
    );
}
