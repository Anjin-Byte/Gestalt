//! GPU **deferred**-truecolor render test (Stage A·1).
//!
//! Builds a per-voxel truecolor-baked scene, renders it through GTAO's deferred
//! pipeline (G-buffer albedo → composite, gated by `dims.w` bit7) and reads the
//! framebuffer back. Proves (a) the truecolor G-buffer + composite shaders
//! **naga-validate** (they only build when `GpuRenderer::new` runs the truecolor
//! branch), and (b) the per-voxel baked colour reaches the screen, lit by the
//! deferred chain — distinguishable from the position-hash fallback.
//!
//! Unlike the forward path, the deferred path *lights* the albedo, so we assert the
//! baked **hue** dominates rather than exact bytes: a saturated-red bake has
//! green/blue ≈ 0, so after sRGB→linear + lighting the output is strongly
//! red-dominant — whereas the position-hash fallback yields balanced channels. A
//! no-colour control renders the same geometry and must NOT be red-dominant.
//!
//! Gated like `differential.rs`: with no adapter it skips, unless
//! `VOXEL_REQUIRE_GPU=1` forces a hard failure.

#![allow(clippy::cast_precision_loss)]

use voxel_core::fixtures::Solid;
use voxel_core::{MaterialTable, Resolution, SchoolBBuffer, SparseTree, VoxelCoord};
use voxel_gpu::{GpuCamera, GpuContext, GpuError, GpuRenderer, OUTPUT_FORMAT};

/// Saturated red — green/blue near zero so the truecolor albedo is unmistakable
/// after lighting (the position-hash fallback yields balanced channels).
const RED: [u8; 4] = [220, 12, 12, 255];

fn require_gpu() -> bool {
    std::env::var_os("VOXEL_REQUIRE_GPU").is_some()
}

fn context_or_skip() -> Option<GpuContext> {
    match GpuContext::try_new() {
        Ok(ctx) => Some(ctx),
        Err(GpuError::NoAdapter) if !require_gpu() => {
            eprintln!("skip: no GPU adapter (set VOXEL_REQUIRE_GPU=1 to require one)");
            None
        }
        Err(e) => panic!("GPU unavailable: {e}"),
    }
}

/// Perspective camera looking straight down +Z at the centre of an `n³` grid.
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

/// Renders `renderer` from `camera` into a `dim×dim` framebuffer, read back as
/// row-major RGBA8. `dim*4` must be a multiple of 256.
fn read_render(
    ctx: &GpuContext,
    renderer: &mut GpuRenderer,
    camera: &GpuCamera,
    dim: u32,
) -> Vec<[u8; 4]> {
    let tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("truecolor test output"),
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
    let readback = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("truecolor test readback"),
        size: u64::from(dim * dim * 4),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    renderer.render(&mut encoder, camera, &view, dim, dim);
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
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
    ctx.queue.submit(std::iter::once(encoder.finish()));

    let slice = readback.slice(..);
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
    readback.unmap();
    px
}

/// Number of *strongly* red-dominant pixels (`R > G+40 && R > B+40`) in the central
/// window of a `dim×dim` image. The +40 margin separates truecolor (G,B ≈ 0) from
/// the position-hash fallback (balanced channels in the central region).
fn central_red_dominant(px: &[[u8; 4]], dim: u32) -> (usize, usize) {
    let d = dim as usize;
    let (lo, hi) = (d * 3 / 8, d * 5 / 8);
    let mut red = 0;
    let mut total = 0;
    for y in lo..hi {
        for x in lo..hi {
            let p = px[y * d + x];
            total += 1;
            if i32::from(p[0]) > i32::from(p[1]) + 40 && i32::from(p[0]) > i32::from(p[2]) + 40 {
                red += 1;
            }
        }
    }
    (red, total)
}

#[test]
fn deferred_truecolor_albedo_reaches_the_screen() {
    let Some(ctx) = context_or_skip() else { return };
    // sRGB byte-order guard: a flip to an sRGB target would change the decode.
    assert_eq!(OUTPUT_FORMAT, wgpu::TextureFormat::Rgba8Unorm);

    let r = Resolution::new(32).unwrap();
    let tree = SparseTree::build(&Solid { resolution: r });
    let dim = 64u32; // dim*4 = 256, a valid readback bytes_per_row

    // Truecolor: a Solid cube baked saturated red → routes through dims.w bit7.
    let mut colored = SchoolBBuffer::from_sparse(&tree);
    colored.assemble_leaf_color(&tree, |_| RED);
    assert!(
        colored.has_leaf_color(),
        "baked scene must route through truecolor"
    );
    // Constructing this naga-validates the truecolor g-buffer + composite pipelines.
    let mut tc =
        GpuRenderer::new(&ctx, &colored, &voxel_core::MaterialTable::missing_only()).unwrap();
    let tc_px = read_render(&ctx, &mut tc, &front_camera(r, dim), dim);
    let (tc_red, total) = central_red_dominant(&tc_px, dim);

    // Control: the SAME geometry with no baked colour → position-hash fallback.
    let plain = SchoolBBuffer::from_sparse(&tree);
    assert!(!plain.has_leaf_color());
    let mut ctrl =
        GpuRenderer::new(&ctx, &plain, &voxel_core::MaterialTable::missing_only()).unwrap();
    let ctrl_px = read_render(&ctx, &mut ctrl, &front_camera(r, dim), dim);
    let (ctrl_red, _) = central_red_dominant(&ctrl_px, dim);

    assert!(
        tc_red > total / 2,
        "truecolor: the red bake should dominate the central hit region — only \
         {tc_red}/{total} pixels were strongly red-dominant"
    );
    assert!(
        ctrl_red < total / 4,
        "position-hash control should not be red-dominant ({ctrl_red}/{total}); the \
         redness in the truecolor render ({tc_red}/{total}) must come from the albedo path"
    );
}

/// Number of *strongly* red-dominant pixels over the WHOLE image — the
/// chunk-select voxel is a few pixels, so no central window.
fn red_dominant_anywhere(px: &[[u8; 4]]) -> usize {
    px.iter()
        .filter(|p| {
            i32::from(p[0]) > i32::from(p[1]) + 40 && i32::from(p[0]) > i32::from(p[2]) + 40
        })
        .count()
}

#[test]
fn truecolor_chunk_select_reads_a_high_chunk() {
    // Forced-tiny per_chunk=2 drives the N>1 cross-chunk path on a 3-voxel scene
    // (no 285 MiB needed). Three voxels in ONE brick get g = 0,1,2; the visible
    // voxel at local (3,3,0) (morton 27) has rank 2 ⇒ g=2 ⇒ chunk 1. Deferred
    // lighting shades the albedo, so the oracle is hue dominance: the g=2 voxel
    // is the only SATURATED-RED albedo in the scene (its brick-mates are dark
    // blue-leaning, the sky is dark blue) — any strongly red-dominant pixel
    // proves `read_leaf_color` crossed into chunk 1.
    let Some(ctx) = context_or_skip() else { return };
    let r = Resolution::new(32).unwrap();

    // Brick (2,2,0): locals (0,0,0)=m0, (1,0,0)=m1, (3,3,0)=m27 — ascending morton,
    // so the (19,19,0) voxel ranks last (g=2). Near grid centre ⇒ clearly visible.
    let vox_g0 = VoxelCoord::new(16, 16, 0);
    let vox_g1 = VoxelCoord::new(17, 16, 0);
    let vox_g2 = VoxelCoord::new(19, 19, 0); // the cross-chunk voxel
    let color_of = move |coord: VoxelCoord| -> [u8; 4] {
        if coord == vox_g2 {
            RED
        } else if coord == vox_g0 {
            [0x11, 0x22, 0x33, 0xFF]
        } else if coord == vox_g1 {
            [0x14, 0x25, 0x66, 0xFF]
        } else {
            [0, 0, 0, 0xFF]
        }
    };

    let tree = SparseTree::from_voxels(r, [vox_g0, vox_g1, vox_g2].map(|v| (v, 0u16)));
    let mut structure = SchoolBBuffer::from_sparse(&tree);
    structure.assemble_leaf_color(&tree, color_of);
    assert_eq!(
        structure.leaf_color_words().len(),
        3,
        "exactly 3 occupied voxels"
    );

    // CPU precondition: the visible voxel actually crosses a chunk boundary,
    // else the test is vacuous (chunk-select never leaves chunk 0).
    let slot = tree.leaf_slot_of(vox_g2).unwrap() as usize;
    let morton = voxel_core::morton::encode_brick(vox_g2.x & 7, vox_g2.y & 7, vox_g2.z & 7);
    let g =
        structure.leaf_color_base_words()[slot] + structure.leaves()[slot].occupied_rank(morton);
    assert_eq!(g, 2, "the (19,19,0) voxel must be global index 2");
    let per_chunk = 2u32;
    assert!(
        g / per_chunk >= 1,
        "geometry did not cross a chunk boundary; test is vacuous"
    );

    let mut renderer = GpuRenderer::new_with_per_chunk(
        &ctx,
        &structure,
        &MaterialTable::missing_only(),
        per_chunk,
    )
    .unwrap();
    let dim = 64u32;
    let px = read_render(&ctx, &mut renderer, &front_camera(r, dim), dim);

    let red = red_dominant_anywhere(&px);
    assert!(
        red > 0,
        "the chunk-1 voxel's saturated-red albedo must reach the screen \
         (proves read_leaf_color crossed into chunk 1)"
    );
}

#[test]
fn device_grants_enough_storage_buffers_for_truecolor() {
    // The pure probe can't see the device's GRANTED limit; this confirms the real
    // adapter supplies the 7 storage buffers the truecolor g-buffer binds (3
    // structure + page + 3 chunks). Stock wgpu default is 8, so this passes
    // everywhere the palette path already runs.
    let Some(ctx) = context_or_skip() else { return };
    assert!(
        ctx.max_storage_buffers() >= 7,
        "truecolor binds 7 storage buffers but the device grants only {}",
        ctx.max_storage_buffers()
    );
}
