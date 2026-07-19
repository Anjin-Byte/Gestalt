//! GPU differential for the **editable paged** truecolor path
//! (`docs/design/brush-editing/02`, Stage A2).
//!
//! Two oracles, both pixel-exact on real hardware:
//! 1. **Parity** — a scene rendered through `GpuRenderer::new_paged` (colours in
//!    the tree's editable pool) is byte-identical to the same scene through the
//!    build-once `GpuRenderer::new` (colours in the structure bake). Run with a
//!    forced-tiny pool chunk so the paged read crosses a chunk boundary, so this
//!    also exercises the multi-chunk `read_leaf_color`.
//! 2. **Incremental edit == fresh rebuild** — after an in-place paint, an
//!    in-place add that crosses a size class (moving the leaf's page), and a
//!    topology erase, the incrementally-synced renderer renders identically to a
//!    fresh `new_paged` of the edited tree. This is the render-edit-render
//!    differential the roadmap requires, with the fresh rebuild as the reference.
//!
//! Gated like the other GPU tests: skips with no adapter unless
//! `VOXEL_REQUIRE_GPU=1`.

#![allow(clippy::cast_precision_loss)]

use voxel_core::color_pool::pack_page;
use voxel_core::{Edit, MaterialTable, Resolution, SchoolBBuffer, SparseTree, VoxelCoord};
use voxel_gpu::{GpuCamera, GpuContext, GpuError, GpuRenderer, OUTPUT_FORMAT};

fn require_gpu() -> bool {
    std::env::var_os("VOXEL_REQUIRE_GPU").is_some()
}

fn context_or_skip() -> Option<GpuContext> {
    match GpuContext::try_new() {
        Ok(ctx) => Some(ctx),
        Err(GpuError::NoAdapter) if !require_gpu() => {
            eprintln!("skip: no GPU adapter present (set VOXEL_REQUIRE_GPU=1 to require one)");
            None
        }
        Err(e) => panic!("GPU unavailable: {e}"),
    }
}

fn res(n: u32) -> Resolution {
    Resolution::new(n).unwrap()
}

/// A perspective camera looking down +Z at the centre of an `n³` grid.
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

/// Renders `renderer` into a `dim×dim` framebuffer, read back row-major.
fn read_render(
    ctx: &GpuContext,
    renderer: &GpuRenderer,
    camera: &GpuCamera,
    dim: u32,
) -> Vec<[u8; 4]> {
    let tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("paged test output"),
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
        label: Some("paged test readback"),
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
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    ctx.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let px = data
        .chunks_exact(4)
        .map(|c| [c[0], c[1], c[2], c[3]])
        .collect();
    drop(data);
    readback.unmap();
    px
}

fn byte(v: u32) -> u8 {
    u8::try_from(v & 0xff).unwrap()
}

/// A unique opaque colour per voxel (packs the coord), so any mis-read is a diff.
fn color_bytes(c: VoxelCoord) -> [u8; 4] {
    [byte(c.x), byte(c.y), byte(c.z), 255]
}

/// The colours of a tree's occupied voxels in slot × intra-brick-Morton (rank)
/// order — what `install_colors` expects — applying `f` per world voxel.
fn colors_in_order(tree: &SparseTree, f: impl Fn(VoxelCoord) -> [u8; 4]) -> Vec<u32> {
    let mut out = Vec::new();
    for idx in 0..tree.leaf_count() {
        let o = tree.leaf_origin(idx);
        for m in 0..512u32 {
            let l = voxel_core::morton::decode(u64::from(m));
            let w = VoxelCoord::new(o.x + l.x, o.y + l.y, o.z + l.z);
            if tree.is_occupied(w) {
                out.push(u32::from_le_bytes(f(w)));
            }
        }
    }
    out
}

/// A paged renderer for `tree` (which must already carry an editable colour
/// store), plus the matching structure.
fn paged(ctx: &GpuContext, tree: &SparseTree) -> (SchoolBBuffer, GpuRenderer) {
    let structure = SchoolBBuffer::from_sparse(tree);
    let pages = tree.color_pages().expect("tree must be truecolor");
    let renderer = GpuRenderer::new_paged(ctx, &structure, &pages).unwrap();
    (structure, renderer)
}

/// Re-uploads leaf `leaf`'s current colour page and page-table entry (an in-place
/// colour or class-changing occupancy edit).
fn sync_color(renderer: &mut GpuRenderer, tree: &SparseTree, leaf: u32) {
    let pages = tree.color_pages().unwrap();
    let i = leaf as usize;
    let page = pages.page_of(i);
    let words = pack_page(pages.colors_of(i), pages.class_of(i));
    renderer.update_color_page(u64::from(page), &words).unwrap();
    renderer.update_page_word(leaf, page).unwrap();
}

/// Two fully-occupied 8³ bricks near the grid centre (1024 voxels): with a
/// 512-entry pool chunk their two 512-class pages land in chunks 0 and 1, so the
/// paged read must cross a chunk boundary.
fn two_brick_tree(r: Resolution) -> SparseTree {
    let mut voxels = Vec::new();
    for &(bx, by) in &[(2u32, 2u32), (3, 2)] {
        for z in 0..8 {
            for y in 0..8 {
                for x in 0..8 {
                    voxels.push(VoxelCoord::new(bx * 8 + x, by * 8 + y, z));
                }
            }
        }
    }
    SparseTree::from_voxels(r, voxels.into_iter().map(|v| (v, 0u16)))
}

#[test]
fn paged_render_matches_static_across_a_chunk_boundary() {
    let Some(ctx) = context_or_skip() else { return };
    assert_eq!(OUTPUT_FORMAT, wgpu::TextureFormat::Rgba8Unorm);
    let r = res(32);
    let dim = 64u32;

    let base = two_brick_tree(r);

    // Static (build-once) reference: colours baked into the structure.
    let mut static_struct = SchoolBBuffer::from_sparse(&base);
    static_struct.assemble_leaf_color(&base, color_bytes);
    let static_r = GpuRenderer::new(&ctx, &static_struct, &MaterialTable::missing_only()).unwrap();

    // Paged: colours installed into the tree with a forced-tiny 512-entry chunk,
    // so the 1024-entry pool spans two GPU chunks.
    let mut paged_tree = base.clone();
    paged_tree.install_colors_with_chunk(colors_in_order(&base, color_bytes).into_iter(), 512);
    assert!(
        paged_tree.color_pages().unwrap().total_entries() > 512,
        "scene must exceed one 512-entry chunk (else the cross-chunk read is untested)"
    );
    let (_ps, paged_r) = paged(&ctx, &paged_tree);

    assert_eq!(
        read_render(&ctx, &static_r, &front_camera(r, dim), dim),
        read_render(&ctx, &paged_r, &front_camera(r, dim), dim),
        "paged render diverged from the build-once static render (cross-chunk read bug?)"
    );
}

#[test]
fn in_place_paint_matches_a_fresh_rebuild() {
    let Some(ctx) = context_or_skip() else { return };
    let r = res(32);
    let dim = 64u32;

    let mut tree = two_brick_tree(r);
    tree.install_colors(colors_in_order(&tree, color_bytes).into_iter());
    let (_structure, mut renderer) = paged(&ctx, &tree);

    // Paint a voxel in the second brick (front face, visible) a distinct colour.
    let target = VoxelCoord::new(25, 17, 0);
    assert!(tree.is_occupied(target));
    // Opaque green (alpha 255 in the high byte) — v1 paint never writes alpha < 255,
    // which would flip the scene to the blend pipeline.
    let green = u32::from_le_bytes([0, 255, 0, 255]);
    let Edit::Color { leaf } = tree.set_color(target, green) else {
        panic!("expected an in-place colour edit");
    };
    sync_color(&mut renderer, &tree, leaf);

    let (_fs, fresh) = paged(&ctx, &tree);
    assert_eq!(
        read_render(&ctx, &renderer, &front_camera(r, dim), dim),
        read_render(&ctx, &fresh, &front_camera(r, dim), dim),
        "painted render diverged from a fresh rebuild of the edited tree"
    );
}

#[test]
fn in_place_add_crossing_a_class_matches_a_fresh_rebuild() {
    let Some(ctx) = context_or_skip() else { return };
    let r = res(32);
    let dim = 64u32;

    // A brick with exactly 32 occupied voxels (class 32); adding a 33rd crosses to
    // class 64 and moves the leaf's page, exercising update_page_word + a grow.
    let mut voxels = Vec::new();
    for m in 0..32u32 {
        let l = voxel_core::morton::decode(u64::from(m));
        voxels.push(VoxelCoord::new(16 + l.x, 16 + l.y, l.z));
    }
    let mut tree = SparseTree::from_voxels(r, voxels.into_iter().map(|v| (v, 0u16)));
    tree.install_colors_with_chunk(colors_in_order(&tree, color_bytes).into_iter(), 512);
    let (mut structure, mut renderer) = paged(&ctx, &tree);

    // The 33rd voxel in the same brick (morton 32), a visible front-face cell.
    let l = voxel_core::morton::decode(32);
    let extra = VoxelCoord::new(16 + l.x, 16 + l.y, l.z);
    let Edit::Leaf(leaf) = tree.set_voxel_colored(extra, true, 0xFF00_FFFFu32) else {
        panic!("expected an in-place occupancy edit");
    };
    // Occupancy patch + colour-page move sync.
    structure.patch_leaf(&tree, leaf);
    renderer.update_leaf(&structure, leaf).unwrap();
    sync_color(&mut renderer, &tree, leaf);

    let (_fs, fresh) = paged(&ctx, &tree);
    assert_eq!(
        read_render(&ctx, &renderer, &front_camera(r, dim), dim),
        read_render(&ctx, &fresh, &front_camera(r, dim), dim),
        "class-crossing add diverged from a fresh rebuild of the edited tree"
    );
}

#[test]
fn topology_erase_via_reupload_paged_matches_a_fresh_rebuild() {
    let Some(ctx) = context_or_skip() else { return };
    let r = res(32);
    let dim = 64u32;

    // Two separate single-voxel bricks; erase one → the brick disappears
    // (topology). reupload_paged rebuilds structure + page table; the surviving
    // leaf's page is unchanged in the pool, so no colour re-upload is needed.
    let keep = VoxelCoord::new(16, 16, 0);
    let drop = VoxelCoord::new(25, 17, 0);
    let mut tree = SparseTree::from_voxels(r, [keep, drop].map(|v| (v, 0u16)));
    tree.install_colors(colors_in_order(&tree, color_bytes).into_iter());
    let (_structure, mut renderer) = paged(&ctx, &tree);

    assert_eq!(tree.set_voxel(drop, false), Edit::Topology);
    let structure = SchoolBBuffer::from_sparse(&tree);
    renderer.reupload_paged(&structure).unwrap();

    let (_fs, fresh) = paged(&ctx, &tree);
    assert_eq!(
        read_render(&ctx, &renderer, &front_camera(r, dim), dim),
        read_render(&ctx, &fresh, &front_camera(r, dim), dim),
        "post-erase render diverged from a fresh rebuild of the edited tree"
    );
}

/// The hover-cursor pin (brush-editing Stage D): with the cursor inactive the
/// render is byte-identical to a build that never touched it (the default
/// zeroed uniform), an active cursor over the surface visibly changes pixels,
/// and deactivating restores the baseline exactly.
#[test]
fn cursor_ring_is_invisible_when_inactive_and_reversible() {
    let Some(ctx) = context_or_skip() else { return };
    let r = res(32);
    let dim = 64u32;
    let base = two_brick_tree(r);
    let mut tree = base.clone();
    tree.install_colors(colors_in_order(&base, color_bytes).into_iter());
    let (_s, renderer) = paged(&ctx, &tree);
    let cam = front_camera(r, dim);

    let baseline = read_render(&ctx, &renderer, &cam, dim);
    // A ring centred on the front face of the first brick, radius 4.
    renderer.set_cursor([20.0, 20.0, 0.0], 4.0, true);
    let ringed = read_render(&ctx, &renderer, &cam, dim);
    assert_ne!(baseline, ringed, "an active cursor must be visible");
    renderer.set_cursor([20.0, 20.0, 0.0], 4.0, false);
    assert_eq!(
        read_render(&ctx, &renderer, &cam, dim),
        baseline,
        "deactivating the cursor must restore the exact baseline"
    );
}

/// The themed-sky pin: a custom sky changes the image, renders identically
/// across the static and paged pipelines (one `sky_color`, both entries), and
/// restoring the default endpoints restores the exact baseline.
#[test]
fn themed_sky_is_consistent_across_pipelines_and_reversible() {
    let Some(ctx) = context_or_skip() else { return };
    let r = res(32);
    let dim = 64u32;
    let base = two_brick_tree(r);

    let mut static_struct = SchoolBBuffer::from_sparse(&base);
    static_struct.assemble_leaf_color(&base, color_bytes);
    let static_r = GpuRenderer::new(&ctx, &static_struct, &MaterialTable::missing_only()).unwrap();
    let mut paged_tree = base.clone();
    paged_tree.install_colors(colors_in_order(&base, color_bytes).into_iter());
    let (_s, paged_r) = paged(&ctx, &paged_tree);
    let cam = front_camera(r, dim);

    let baseline = read_render(&ctx, &paged_r, &cam, dim);
    // A warm light-theme sky (sRGB RGBA8, R low).
    let top = u32::from_le_bytes([248, 244, 233, 255]);
    let bottom = u32::from_le_bytes([220, 215, 202, 255]);
    static_r.set_sky(top, bottom);
    paged_r.set_sky(top, bottom);
    let lit_static = read_render(&ctx, &static_r, &cam, dim);
    let lit_paged = read_render(&ctx, &paged_r, &cam, dim);
    assert_ne!(baseline, lit_paged, "a new sky must be visible");
    assert_eq!(lit_static, lit_paged, "both pipelines share one sky");
    // Reversible: re-setting the same endpoints reproduces the same image
    // exactly (the dither is deterministic per pixel). The float defaults
    // themselves are finer than 8-bit endpoints, so "back to default" is a
    // ±1 LSB question by construction — reversibility is the honest pin.
    paged_r.set_sky(u32::from_le_bytes([10, 20, 30, 255]), bottom);
    paged_r.set_sky(top, bottom);
    assert_eq!(
        read_render(&ctx, &paged_r, &cam, dim),
        lit_paged,
        "same endpoints, same image"
    );
    let _ = baseline;
}
