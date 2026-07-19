//! GPU **deferred**-palette render test (Stage B).
//!
//! Builds a material-indexed scene (no baked colour → the renderer's `Palette`
//! albedo mode), renders it through GTAO's deferred pipeline (palette g-buffer
//! variant → `gbuf_albedo` → composite, gated by `dims.w` bit2) and reads the
//! framebuffer back. Proves (a) the palette g-buffer + `palette_lookup.wgsl`
//! (`read_material`) naga-validate, and (b) per-voxel **material** colour reaches
//! the screen, lit by the deferred chain.
//!
//! The scene splits each `8³` leaf into a red half and a green half (`x % 8 < 4`),
//! so every leaf carries **two** palette entries ⇒ `bits_per_voxel > 0` — exercising
//! `read_material`'s index-decode path (the bits==0 single-material fast path tests
//! nothing of it). We assert both lit hues appear in the central region; the
//! deferred path *lights* the albedo, so we check hue-dominance, not exact bytes.
//!
//! Gated like `differential.rs`: with no adapter it skips, unless
//! `VOXEL_REQUIRE_GPU=1` forces a hard failure.

#![allow(clippy::cast_precision_loss)]

use voxel_core::fixtures::Solid;
use voxel_core::{MaterialTable, Resolution, SchoolBBuffer, SparseTree};
use voxel_gpu::{GpuCamera, GpuContext, GpuError, GpuRenderer, OUTPUT_FORMAT};

/// Pure red / green as packed `u32` (R in the low byte, matching `unpack4x8unorm`).
const RED: u32 = 0xFF00_00FF;
const GREEN: u32 = 0xFF00_FF00;

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
        label: Some("palette test output"),
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
        label: Some("palette test readback"),
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

/// Count *strongly* red- and green-dominant pixels in the central window. The +40
/// margin separates a saturated lit palette colour from any balanced fallback.
fn central_hues(px: &[[u8; 4]], dim: u32) -> (usize, usize, usize) {
    let d = dim as usize;
    let (lo, hi) = (d * 3 / 8, d * 5 / 8);
    let (mut red, mut green, mut total) = (0, 0, 0);
    for y in lo..hi {
        for x in lo..hi {
            let [r, g, b, _] = px[y * d + x];
            let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
            total += 1;
            if r > g + 40 && r > b + 40 {
                red += 1;
            } else if g > r + 40 && g > b + 40 {
                green += 1;
            }
        }
    }
    (red, green, total)
}

#[test]
fn deferred_palette_materials_reach_the_screen_via_index_decode() {
    let Some(ctx) = context_or_skip() else { return };
    assert_eq!(OUTPUT_FORMAT, wgpu::TextureFormat::Rgba8Unorm);

    let r = Resolution::new(32).unwrap();
    let dim = 64u32; // dim*4 = 256, a valid readback bytes_per_row

    // Solid cube, each 8³ leaf split red|green within the leaf (x%8 < 4) so every
    // leaf has a 2-entry palette ⇒ bits_per_voxel > 0 (exercises the index decode).
    let mut tree = SparseTree::build(&Solid { resolution: r });
    tree.fill_materials(|c| if c.x % 8 < 4 { 1 } else { 2 });
    let mut table = MaterialTable::missing_only();
    assert_eq!(table.push(RED).unwrap(), 1, "red gets global id 1");
    assert_eq!(table.push(GREEN).unwrap(), 2, "green gets global id 2");

    let structure = SchoolBBuffer::from_sparse(&tree);
    assert!(
        !structure.has_leaf_color() && !structure.leaf_mat_words().is_empty(),
        "scene must route through the Palette albedo mode (materials, no baked colour)"
    );

    // Constructing this naga-validates the palette g-buffer + palette_lookup.wgsl.
    let mut renderer = GpuRenderer::new(&ctx, &structure, &table).unwrap();
    let px = read_render(&ctx, &mut renderer, &front_camera(r, dim), dim);
    let (red, green, total) = central_hues(&px, dim);

    // Both palette entries must reach the screen — red AND green stripes are lit and
    // present. Only a correct `read_material` index-decode produces both saturated
    // hues; a mis-decode collapses to one colour, the magenta sentinel, or garbage.
    assert!(
        red > total / 10,
        "red material should fill ~half the striped central region; only {red}/{total} red-dominant"
    );
    assert!(
        green > total / 10,
        "green material (the 2nd palette entry, bits>0 index path) should appear; only {green}/{total} green-dominant"
    );
}

/// Strongly magenta-dominant (R and B high, G low) — the lit MISSING sentinel's
/// signature under deferred shading.
fn magenta_dominant(px: &[[u8; 4]]) -> usize {
    px.iter()
        .filter(|p| {
            i32::from(p[0]) > i32::from(p[1]) + 40 && i32::from(p[2]) > i32::from(p[1]) + 40
        })
        .count()
}

#[test]
fn gpu_unassigned_material_falls_back_to_position_not_magenta() {
    // With no materials assigned, every hit reads global-0; the deferred
    // composite shades those by position (the prior fixture look), NOT the
    // magenta sentinel. Guards the gid==0 fallback so adding materials never
    // regresses fixture rendering.
    let Some(ctx) = context_or_skip() else { return };
    let r = Resolution::new(32).unwrap();
    let tree = SparseTree::build(&Solid { resolution: r }); // no fill_materials ⇒ all gid 0
    let table = MaterialTable::missing_only();
    let structure = SchoolBBuffer::from_sparse(&tree);

    let dim = 64u32;
    let mut renderer = GpuRenderer::new(&ctx, &structure, &table).unwrap();
    let px = read_render(&ctx, &mut renderer, &front_camera(r, dim), dim);

    assert_eq!(
        magenta_dominant(&px),
        0,
        "global-0 hits shade by position, never the magenta sentinel"
    );
    // Position shading varies across the cube — many distinct hit colours,
    // unlike a single-material scene.
    let distinct: std::collections::HashSet<[u8; 4]> = px.iter().copied().collect();
    assert!(
        distinct.len() > 3,
        "position shading should produce a varied hit region, got {} colours",
        distinct.len()
    );
}
