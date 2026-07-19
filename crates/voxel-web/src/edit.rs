//! The effectful stroke controller (`docs/design/brush-editing/04`, Stage C):
//! cast the render kernel's exact ray from a cursor position, resample the
//! stroke into stamps, ask the pure `voxel-brush` kernels for ops, apply them
//! to the tree with the undo journal capturing at the seam, and bring the
//! CPU-side School-B buffer into step (in-place leaf patches, or a full
//! rebuild when the stroke changed topology). GPU sync stays in the engine.
//!
//! All brush *math* lives in `voxel-brush` (pure, natively tested against
//! brute-force references); everything here is the mutation plumbing.

use glam::{DVec3, Vec3};
use voxel_brush::{
    BrushParams, Field, Plane, Stamp, StrokeMask, VoxelOp, estimate_normal, mirrored, resample,
    stamp_ops,
};
use voxel_core::{Edit, GpuCamera, Ray, SchoolBBuffer, SparseTree, VoxelCoord, morton};

use crate::undo::StrokeJournal;

/// The mutable state one stroke accumulates: the paint mask (cleared at
/// `brush_end`), the undo journal (committed into the ring there), the
/// stroke's **anchor plane** (position + gradient normal at the first hit,
/// held for the stroke's lifetime — Flatten's facet plane, and the plane
/// self-hit picks deflect onto), and the set of voxels this stroke added
/// (the self-hit mask — see [`resolve_pick`]).
#[derive(Default)]
pub(crate) struct StrokeState {
    pub(crate) mask: StrokeMask,
    pub(crate) journal: StrokeJournal,
    pub(crate) anchor: Option<Plane>,
    /// Voxels whose occupancy this stroke *created*. A pick landing on one is
    /// the stroke re-hitting its own fresh material — the depth-march that
    /// pillars toward the camera — and is deflected onto the anchor plane.
    pub(crate) added: std::collections::HashSet<VoxelCoord>,
}

/// What a brush stroke did — the engine picks the GPU sync strategy from it.
pub(crate) struct BrushOutcome {
    /// Voxels actually changed (no-ops within the brush don't count).
    pub(crate) changed: u32,
    /// Whether the stroke changed topology (leaf indices renumbered — the
    /// structure was rebuilt and needs a full re-upload).
    pub(crate) topology: bool,
    /// Deduped touched leaf slots for the occupancy patch (meaningful only when
    /// `!topology`; a topology stroke rebuilds the whole structure).
    pub(crate) touched: Vec<u32>,
    /// Deduped **brick Morton codes** whose colour changed — stable across the
    /// topology renumber, so the engine maps each back to its current slot to
    /// re-upload one colour page. Empty on a non-truecolor scene.
    pub(crate) color_bricks: Vec<u64>,
}

/// Casts the ray for pixel `(px, py)` in a `w×h` viewport through `cam` —
/// identical to `render.wgsl`'s ray-gen (NDC → camera-basis direction), so the
/// edit lands exactly where the pixel was drawn.
pub(crate) fn cursor_ray(cam: &GpuCamera, px: f32, py: f32, w: f32, h: f32) -> Ray {
    let ndc_x = ((px + 0.5) / w * 2.0 - 1.0) * cam.tan * cam.aspect;
    let ndc_y = (1.0 - (py + 0.5) / h * 2.0) * cam.tan;
    let dir = (Vec3::from_array(cam.forward)
        + Vec3::from_array(cam.right) * ndc_x
        + Vec3::from_array(cam.up) * ndc_y)
        .normalize();
    Ray::new(Vec3::from_array(cam.eye).as_dvec3(), dir.as_dvec3())
}

/// The read-only `Field` view of a tree the pure kernels sample.
struct TreeField<'a> {
    tree: &'a SparseTree,
}

impl Field for TreeField<'_> {
    fn occupied(&self, x: i64, y: i64, z: i64) -> bool {
        let (Ok(x), Ok(y), Ok(z)) = (u32::try_from(x), u32::try_from(y), u32::try_from(z)) else {
            return false; // off-grid probes are unoccupied
        };
        self.tree.is_occupied(VoxelCoord::new(x, y, z))
    }

    fn color(&self, x: i64, y: i64, z: i64) -> u32 {
        let (Ok(x), Ok(y), Ok(z)) = (u32::try_from(x), u32::try_from(y), u32::try_from(z)) else {
            return 0xFF00_0000;
        };
        self.tree
            .color_at(VoxelCoord::new(x, y, z))
            .unwrap_or(0xFF00_0000)
    }
}

/// The brick Morton code of the voxel at `c`.
fn brick_code(c: VoxelCoord) -> u64 {
    morton::encode(c.x >> 3, c.y >> 3, c.z >> 3)
}

/// One pointer event of a stroke, as the engine saw it: the picked voxel, the
/// pen pressure, and the view direction fallback that orients Flatten's anchor
/// when the surface is degenerate (an isolated voxel — the brush faces the
/// viewer, `-ray.dir`).
#[derive(Clone, Copy)]
pub(crate) struct StrokeEvent {
    pub(crate) hit: VoxelCoord,
    pub(crate) pressure: f32,
    pub(crate) fallback_normal: DVec3,
}

/// Applies one pointer event's worth of a brush stroke with `params`, then
/// **one** sync of `structure` with `tree` — a topology-changing event
/// rebuilds once; in-place events patch the touched leaves' occupancy/material
/// and colour-page words. `prev` is the previous event's stamp (the resample
/// anchor). Returns the changes for the engine to mirror onto the GPU.
///
/// Every mutation flows through the op-application loop below, and each op's
/// brick is journalled **before** it applies — the undo system's single
/// capture seam, which no tool (present or future) can bypass.
pub(crate) fn apply_stroke(
    tree: &mut SparseTree,
    structure: &mut SchoolBBuffer,
    params: &BrushParams,
    prev: Option<Stamp>,
    event: StrokeEvent,
    state: &mut StrokeState,
) -> BrushOutcome {
    let StrokeEvent {
        hit,
        pressure,
        fallback_normal,
    } = event;
    let colored = tree.has_colors();
    let mut touched: Vec<u32> = Vec::new();
    let mut color_bricks: Vec<u64> = Vec::new();
    let mut topology = false;
    let mut changed = 0u32;

    // The stroke's anchor: position + gradient normal at the first hit, held
    // for the stroke's lifetime ([04 §anchor-plane]). Flatten's facet plane,
    // and the plane self-hit picks deflect onto (every tool needs it for
    // that, so it is captured unconditionally — once per stroke).
    if state.anchor.is_none() {
        let normal = estimate_normal(&TreeField { tree }, hit, params.radius)
            .unwrap_or(fallback_normal)
            .normalize_or(DVec3::Z);
        state.anchor = Some(Plane {
            point: DVec3::new(f64::from(hit.x), f64::from(hit.y), f64::from(hit.z)),
            normal,
        });
    }

    let next = Stamp {
        center: hit,
        pressure,
    };
    // The symmetry pre-pass is identity plumbing in v1 (the mirror plane UX
    // ships v1.5; the stamp list is already the right shape).
    let stamps = mirrored(resample(prev, next, params.radius), None);

    for stamp in stamps {
        // Ops are computed against the pre-stamp tree; the borrow ends before
        // application, and the next stamp sees this stamp's writes.
        let ops = stamp_ops(
            &TreeField { tree },
            params,
            &stamp,
            state.anchor,
            &mut state.mask,
        );
        for op in ops {
            state.journal.capture(tree, brick_code(op.coord()));
            let edit = apply_op(tree, colored, op);
            // A Set that changed occupancy is fresh stroke material: record it
            // for the self-hit pick mask ([`resolve_pick`]).
            if matches!(op, VoxelOp::Set { .. }) && matches!(edit, Edit::Leaf(_) | Edit::Topology) {
                state.added.insert(op.coord());
            }
            match edit {
                Edit::Unchanged => {}
                Edit::Leaf(idx) => {
                    touched.push(idx);
                    changed += 1;
                    if colored {
                        color_bricks.push(brick_code(op.coord()));
                    }
                }
                Edit::Topology => {
                    topology = true;
                    changed += 1;
                    if colored {
                        color_bricks.push(brick_code(op.coord()));
                    }
                }
                Edit::Color { .. } => {
                    changed += 1;
                    color_bricks.push(brick_code(op.coord()));
                }
                Edit::Material { .. } => unreachable!("brush never returns Material"),
            }
        }
    }

    color_bricks.sort_unstable();
    color_bricks.dedup();

    if changed > 0 {
        if topology {
            // Topology renumbers leaf indices and invalidates node offsets:
            // re-serialize once (the engine follows with a full re-upload). The
            // colour-page table + transparency bits are re-derived here too.
            *structure = SchoolBBuffer::from_sparse(tree);
            touched.clear();
        } else {
            touched.sort_unstable();
            touched.dedup();
            for &idx in &touched {
                structure.patch_leaf(tree, idx);
                if !colored {
                    // Palette occupancy add defaults to global-0; keep the slot
                    // in step. (Truecolor scenes carry no `leaf_mat` to patch.)
                    structure.patch_leaf_mat(tree, idx);
                }
            }
            // Keep the derived colour-page offsets in step for any brick whose
            // colour store moved (a class-crossing add), so a later topology
            // reupload rebuilds a correct page table.
            for &code in &color_bricks {
                if let Some(slot) = slot_of_brick(tree, code) {
                    structure.patch_leaf_color_page(tree, slot);
                }
            }
        }
    }
    BrushOutcome {
        changed,
        topology,
        touched,
        color_bricks,
    }
}

/// Resolves one event's pick: a hit on pre-existing surface passes through
/// unchanged, but a hit on a voxel **this stroke added** — the stroke
/// re-picking its own fresh material, which otherwise marches the stamp
/// centre toward the camera ("pillaring" while building volume along a
/// surface) — deflects onto the stroke's anchor plane, so the stamp slides
/// *along* the surface the stroke started on. Hits on real surface keep
/// tracking curved geometry truly; only self-hits deflect. Degenerate
/// geometry (no anchor yet, a ray parallel to the plane, an intersection
/// behind the eye) falls back to the raw hit rather than inventing a
/// position; the projection clamps to the grid.
pub(crate) fn resolve_pick(state: &StrokeState, ray: &Ray, hit: VoxelCoord, n: u32) -> VoxelCoord {
    if !state.added.contains(&hit) {
        return hit;
    }
    let Some(plane) = state.anchor else {
        return hit;
    };
    let denom = ray.dir.dot(plane.normal);
    if denom.abs() < 1e-9 {
        return hit;
    }
    let t = (plane.point - ray.origin).dot(plane.normal) / denom;
    if t <= 0.0 {
        return hit;
    }
    let p = (ray.origin + ray.dir * t).round();
    let max = f64::from(n.saturating_sub(1));
    #[allow(clippy::cast_sign_loss)] // clamped to [0, n)
    VoxelCoord::new(
        p.x.clamp(0.0, max) as u32,
        p.y.clamp(0.0, max) as u32,
        p.z.clamp(0.0, max) as u32,
    )
}

/// The Rust transcription of `render.wgsl`'s position shading (each channel
/// `world / n`, written through an rgba8unorm store — unorm quantization is
/// `round(clamp(v, 0, 1) · 255)`): the colour a global-0 voxel showed on
/// screen, so a promoted fixture is pixel-continuous with what was rendered —
/// no visual pop on first paint. sRGB RGBA8, R low, opaque.
pub(crate) fn position_shade(c: VoxelCoord, n: u32) -> u32 {
    let n = f64::from(n.max(1));
    let ch = |v: u32| -> u32 {
        #[allow(clippy::cast_sign_loss)] // clamped non-negative
        let q = ((f64::from(v) / n).clamp(0.0, 1.0) * 255.0).round() as u32;
        q
    };
    ch(c.x) | (ch(c.y) << 8) | (ch(c.z) << 16) | (0xFF << 24)
}

/// The promotion colour map (`docs/design/brush-editing/06 §promotion`):
/// palette voxels take their global-table colour, global-0 voxels keep the
/// position shade they showed on screen.
pub(crate) fn promotion_color(
    table: &voxel_core::MaterialTable,
    n: u32,
    c: VoxelCoord,
    gid: u16,
) -> u32 {
    if gid == 0 {
        position_shade(c, n)
    } else {
        table.color(gid)
    }
}

/// Applies one kernel op to the tree — the single mutation seam (the journal
/// capture precedes this call). Colour-only ops are inert without a store.
fn apply_op(tree: &mut SparseTree, colored: bool, op: VoxelOp) -> Edit {
    match op {
        VoxelOp::Set { c, rgba } => {
            if colored {
                tree.set_voxel_colored(c, true, rgba)
            } else {
                tree.set_voxel(c, true)
            }
        }
        VoxelOp::Clear { c } => tree.set_voxel(c, false),
        VoxelOp::Recolor { c, rgba } => {
            if colored {
                tree.set_color(c, rgba)
            } else {
                Edit::Unchanged
            }
        }
    }
}

/// The leaf slot of the brick with Morton code `code`, or `None` if the brick is
/// gone (erased). Maps a stable brick code back to its (possibly renumbered)
/// slot after a stroke.
pub(crate) fn slot_of_brick(tree: &SparseTree, code: u64) -> Option<u32> {
    let b = morton::decode(code);
    tree.leaf_slot_of(VoxelCoord::new(b.x * 8, b.y * 8, b.z * 8))
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxel_brush::{BrushTool, Falloff};
    use voxel_core::Resolution;

    fn params(tool: BrushTool, radius: u32) -> BrushParams {
        BrushParams {
            tool,
            radius,
            strength: 1.0,
            falloff: Falloff::Smooth,
            color: 0xFFFF_FFFF,
            invert: false,
        }
    }

    fn stroke_event(
        tree: &mut SparseTree,
        structure: &mut SchoolBBuffer,
        p: &BrushParams,
        prev: Option<Stamp>,
        hit: VoxelCoord,
        state: &mut StrokeState,
    ) -> BrushOutcome {
        let event = StrokeEvent {
            hit,
            pressure: 1.0,
            fallback_normal: DVec3::Z,
        };
        apply_stroke(tree, structure, p, prev, event, state)
    }

    /// Runs an occupancy stroke (Draw/Erase) with fresh stroke state.
    fn occ(
        tree: &mut SparseTree,
        structure: &mut SchoolBBuffer,
        prev: Option<VoxelCoord>,
        hit: VoxelCoord,
        radius: u32,
        add: bool,
    ) -> BrushOutcome {
        let p = params(
            if add {
                BrushTool::Draw
            } else {
                BrushTool::Erase
            },
            radius,
        );
        let prev = prev.map(|center| Stamp {
            center,
            pressure: 1.0,
        });
        stroke_event(tree, structure, &p, prev, hit, &mut StrokeState::default())
    }

    fn scene() -> (SparseTree, SchoolBBuffer) {
        let resolution = Resolution::new(32).expect("legal");
        let tree = SparseTree::from_voxels(
            resolution,
            (8..16u32).flat_map(|x| {
                (8..16u32)
                    .flat_map(move |y| (8..16u32).map(move |z| (VoxelCoord::new(x, y, z), 1u16)))
            }),
        );
        let structure = SchoolBBuffer::from_sparse(&tree);
        (tree, structure)
    }

    /// A coloured block scene for the tool sweep.
    fn colored_scene() -> (SparseTree, SchoolBBuffer) {
        let r = Resolution::new(32).expect("legal");
        let mut tree = SparseTree::from_voxels(
            r,
            (8..24u32).flat_map(|x| {
                (8..24u32)
                    .flat_map(move |y| (8..16u32).map(move |z| (VoxelCoord::new(x, y, z), 0u16)))
            }),
        );
        let occ = usize::try_from(tree.occupied_voxels()).unwrap();
        tree.install_colors((0..occ).map(|i| 0xFF00_0000 | u32::try_from(i % 0xFFFF).unwrap()));
        let structure = SchoolBBuffer::from_sparse(&tree);
        (tree, structure)
    }

    /// After any stroke, the patched structure must equal a from-scratch rebuild.
    fn assert_in_step(tree: &SparseTree, structure: &SchoolBBuffer) {
        let rebuilt = SchoolBBuffer::from_sparse(tree);
        assert_eq!(structure.nodes(), rebuilt.nodes());
        assert_eq!(structure.leaves(), rebuilt.leaves());
        assert_eq!(structure.leaf_mat_words(), rebuilt.leaf_mat_words());
        assert_eq!(structure.leaf_bounds_words(), rebuilt.leaf_bounds_words());
        assert_eq!(
            structure.leaf_color_page_words(),
            rebuilt.leaf_color_page_words()
        );
    }

    #[test]
    fn in_place_erase_patches_touched_leaves() {
        let (mut tree, mut structure) = scene();
        let before = tree.occupied_voxels();
        let out = occ(
            &mut tree,
            &mut structure,
            None,
            VoxelCoord::new(12, 12, 12),
            1,
            false,
        );
        assert!(out.changed > 0);
        assert!(!out.topology, "interior erase must stay in-place");
        assert!(!out.touched.is_empty());
        assert!(tree.occupied_voxels() < before);
        assert_in_step(&tree, &structure);
    }

    #[test]
    fn add_into_empty_space_changes_topology() {
        let (mut tree, mut structure) = scene();
        let out = occ(
            &mut tree,
            &mut structure,
            None,
            VoxelCoord::new(28, 28, 28),
            2,
            true,
        );
        assert!(out.changed > 0);
        assert!(
            out.topology,
            "planting bricks in empty space is topological"
        );
        assert_in_step(&tree, &structure);
    }

    #[test]
    fn interpolated_stroke_is_gap_free() {
        let resolution = Resolution::new(32).expect("legal");
        let mut tree = SparseTree::from_voxels(resolution, std::iter::empty());
        let mut structure = SchoolBBuffer::from_sparse(&tree);
        let prev = VoxelCoord::new(2, 10, 10);
        let hit = VoxelCoord::new(18, 10, 10);
        occ(&mut tree, &mut structure, None, prev, 2, true);
        occ(&mut tree, &mut structure, Some(prev), hit, 2, true);
        for x in 2..=18u32 {
            assert!(tree.is_occupied(VoxelCoord::new(x, 10, 10)), "gap at x={x}");
        }
        assert_in_step(&tree, &structure);
    }

    #[test]
    fn distant_hits_do_not_bridge() {
        let resolution = Resolution::new(32).expect("legal");
        let mut tree = SparseTree::from_voxels(resolution, std::iter::empty());
        let mut structure = SchoolBBuffer::from_sparse(&tree);
        let prev = VoxelCoord::new(1, 1, 1);
        let hit = VoxelCoord::new(30, 30, 30);
        occ(&mut tree, &mut structure, None, prev, 1, true);
        occ(&mut tree, &mut structure, Some(prev), hit, 1, true);
        assert!(tree.is_occupied(prev));
        assert!(tree.is_occupied(hit));
        assert!(
            !tree.is_occupied(VoxelCoord::new(15, 15, 15)),
            "midpoint must stay empty past the bridge cap"
        );
        assert_in_step(&tree, &structure);
    }

    #[test]
    fn cursor_ray_center_pixel_looks_forward() {
        let cam = GpuCamera {
            eye: [0.0, 0.0, 0.0],
            tan: 0.5,
            forward: [0.0, 0.0, 1.0],
            aspect: 2.0,
            right: [1.0, 0.0, 0.0],
            n: 32.0,
            up: [0.0, 1.0, 0.0],
            pad: 0.0,
            dims: [200, 100, 2, 0],
        };
        let ray = cursor_ray(&cam, 99.5, 49.5, 200.0, 100.0);
        assert!((ray.dir.z - 1.0).abs() < 1e-9, "{:?}", ray.dir);
        assert!(ray.dir.x.abs() < 1e-9 && ray.dir.y.abs() < 1e-9);
    }

    /// THE controller catch-all (09 §kernel-oracles): after strokes of **every**
    /// tool on a coloured scene, the incrementally-patched structure equals a
    /// from-scratch rebuild — occupancy, materials, bounds, and page table.
    #[test]
    fn rebuild_parity_holds_under_every_tool() {
        let tools = [
            BrushTool::Draw,
            BrushTool::Erase,
            BrushTool::Paint,
            BrushTool::Clay,
            BrushTool::Smooth,
            BrushTool::Flatten,
            BrushTool::Inflate,
        ];
        for tool in tools {
            let (mut tree, mut structure) = colored_scene();
            // A bump on the slab so the plane-relative tools have relief to
            // act on (Flatten on a perfectly flat surface is honestly a no-op).
            stroke_event(
                &mut tree,
                &mut structure,
                &params(BrushTool::Draw, 2),
                None,
                VoxelCoord::new(12, 12, 17),
                &mut StrokeState::default(),
            );
            let p = BrushParams {
                tool,
                radius: 3,
                strength: 1.0,
                falloff: Falloff::Smooth,
                color: u32::from_le_bytes([30, 200, 90, 255]),
                invert: false,
            };
            let mut state = StrokeState::default();
            // Two events (the bump, then a drag onto the flat) — so every tool
            // has both relief and surface to act on.
            let first = VoxelCoord::new(12, 12, 15);
            let second = VoxelCoord::new(18, 12, 15);
            let out1 = stroke_event(&mut tree, &mut structure, &p, None, first, &mut state);
            let prev = Stamp {
                center: first,
                pressure: 1.0,
            };
            let out2 = stroke_event(
                &mut tree,
                &mut structure,
                &p,
                Some(prev),
                second,
                &mut state,
            );
            assert_in_step(&tree, &structure);
            // Every tool must actually do something across the two events (a
            // single event may honestly be silent — Flatten's first stamp on
            // the anchor plane, Clay against the fully-occupied bump).
            assert!(out1.changed + out2.changed > 0, "{tool:?} was a no-op");
        }
    }

    /// Flatten's anchor is captured once per stroke: the first event's plane
    /// governs the second event's ops (facets, not local-surface chasing).
    #[test]
    fn flatten_anchor_is_stroke_stable() {
        let (mut tree, mut structure) = colored_scene();
        let p = BrushParams {
            tool: BrushTool::Flatten,
            radius: 4,
            strength: 1.0,
            falloff: Falloff::Smooth,
            color: 0xFFFF_FFFF,
            invert: false,
        };
        let mut state = StrokeState::default();
        let first = VoxelCoord::new(12, 12, 15);
        stroke_event(&mut tree, &mut structure, &p, None, first, &mut state);
        let anchor = state.anchor.expect("anchor captured at stroke start");
        stroke_event(
            &mut tree,
            &mut structure,
            &p,
            Some(Stamp {
                center: first,
                pressure: 1.0,
            }),
            VoxelCoord::new(18, 12, 15),
            &mut state,
        );
        assert_eq!(state.anchor, Some(anchor), "anchor held for the stroke");
        assert_in_step(&tree, &structure);
    }

    /// The colour rebuild-parity oracle from Stage A3, kept: Draw / Paint /
    /// Erase on a truecolor tree stay byte-in-step with a fresh rebuild, and a
    /// drawn voxel carries the brush colour.
    #[test]
    fn colour_strokes_match_a_fresh_rebuild() {
        let (mut tree, mut structure) = colored_scene();
        let mut state = StrokeState::default();
        let paint = BrushParams {
            tool: BrushTool::Paint,
            radius: 4,
            strength: 1.0,
            falloff: Falloff::Smooth,
            color: u32::from_le_bytes([10, 200, 30, 255]),
            invert: false,
        };
        let out = stroke_event(
            &mut tree,
            &mut structure,
            &paint,
            None,
            VoxelCoord::new(15, 15, 12),
            &mut state,
        );
        assert!(out.changed > 0 && !out.topology && !out.color_bricks.is_empty());
        assert_in_step(&tree, &structure);

        let draw = params(BrushTool::Draw, 2);
        stroke_event(
            &mut tree,
            &mut structure,
            &draw,
            None,
            VoxelCoord::new(28, 28, 28),
            &mut StrokeState::default(),
        );
        assert_in_step(&tree, &structure);
        // A drawn voxel carries the brush colour.
        assert_eq!(
            tree.color_at(VoxelCoord::new(28, 28, 28)),
            Some(0xFFFF_FFFF)
        );

        let erase = params(BrushTool::Erase, 1);
        stroke_event(
            &mut tree,
            &mut structure,
            &erase,
            None,
            VoxelCoord::new(15, 15, 12),
            &mut StrokeState::default(),
        );
        assert_in_step(&tree, &structure);
    }

    /// THE anti-pillar oracle (the "building volume along a surface" fix): a
    /// lateral Draw drag over a slab, driven through the real pick loop
    /// (traverse → `resolve_pick` → `apply_stroke`), must lay material *along*
    /// the surface — bounded by one ball height — instead of climbing toward
    /// the camera one fresh-material hit at a time.
    #[test]
    fn lateral_drags_build_along_the_surface_not_toward_the_camera() {
        let (mut tree, mut structure) = colored_scene(); // slab top at z = 15
        let p = params(BrushTool::Draw, 2);
        let mut state = StrokeState::default();
        let mut prev: Option<Stamp> = None;
        for i in 0..8u32 {
            // The camera looks straight down; the cursor slides one voxel in
            // +x per event — dense enough that the raw ray keeps re-hitting
            // the previous stamp's fresh material.
            let eye = DVec3::new(f64::from(10 + i), 12.0, 30.0);
            let ray = Ray::new(eye, DVec3::new(0.0, 0.0, -1.0));
            let hit = voxel_core::traverse(&structure, &ray)
                .expect("over the slab")
                .voxel;
            let picked = resolve_pick(&state, &ray, hit, 32);
            apply_stroke(
                &mut tree,
                &mut structure,
                &p,
                prev,
                StrokeEvent {
                    hit: picked,
                    pressure: 1.0,
                    fallback_normal: DVec3::Z,
                },
                &mut state,
            );
            prev = Some(Stamp {
                center: picked,
                pressure: 1.0,
            });
        }
        let top = tree.occupied_bbox().expect("occupied").1.z;
        assert!(
            top <= 15 + 2,
            "pillared to z = {top} (one ball above the z = 15 surface is 17)"
        );
        assert_in_step(&tree, &structure);
    }

    /// `resolve_pick` passes pre-existing hits through, deflects self-hits
    /// onto the anchor plane, and falls back on degenerate geometry.
    #[test]
    fn resolve_pick_deflects_only_self_hits() {
        let mut state = StrokeState::default();
        let hit = VoxelCoord::new(10, 10, 16);
        let ray = Ray::new(DVec3::new(10.0, 10.0, 30.0), DVec3::new(0.0, 0.0, -1.0));
        // Not the stroke's own material → pass through (also: no anchor yet).
        assert_eq!(resolve_pick(&state, &ray, hit, 32), hit);
        state.anchor = Some(Plane {
            point: DVec3::new(10.0, 10.0, 15.0),
            normal: DVec3::Z,
        });
        state.added.insert(hit);
        // A self-hit lands on the anchor plane below instead.
        assert_eq!(
            resolve_pick(&state, &ray, hit, 32),
            VoxelCoord::new(10, 10, 15)
        );
        // A ray parallel to the plane cannot deflect: raw hit.
        let grazing = Ray::new(DVec3::new(0.0, 10.0, 16.0), DVec3::new(1.0, 0.0, 0.0));
        assert_eq!(resolve_pick(&state, &grazing, hit, 32), hit);
    }

    /// The position-shade parity pin (09 §promotion-oracle): the Rust
    /// transcription matches `render.wgsl`'s `world / n` through rgba8unorm
    /// quantization — exact bytes on known inputs, monotone along each axis.
    #[test]
    fn position_shade_matches_the_wgsl_unorm_formula() {
        let v = VoxelCoord::new;
        assert_eq!(position_shade(v(0, 0, 0), 32), 0xFF00_0000);
        // 16/32 → 0.5 → round(127.5) = 128 on every written channel.
        assert_eq!(
            position_shade(v(16, 16, 16), 32).to_le_bytes(),
            [128, 128, 128, 255]
        );
        // 31/32 → 247.03 → 247; 8/32 → 63.75 → 64.
        assert_eq!(
            position_shade(v(31, 8, 0), 32).to_le_bytes(),
            [247, 64, 0, 255]
        );
        for x in 1..32u32 {
            assert!(
                position_shade(v(x, 0, 0), 32).to_le_bytes()[0]
                    >= position_shade(v(x - 1, 0, 0), 32).to_le_bytes()[0],
                "monotone in x"
            );
        }
    }

    /// The promotion oracle, voxel-web half: after promoting with the real
    /// colour map, palette voxels read their table colour, global-0 voxels
    /// their position shade — and the promoted tree paints like any truecolor
    /// scene, staying in step with a fresh rebuild.
    #[test]
    fn promotion_bakes_table_colours_and_position_shade() {
        let r = Resolution::new(32).expect("legal");
        let n = 32u32;
        let v = VoxelCoord::new;
        let mut table = voxel_core::MaterialTable::missing_only();
        let red = table.push(u32::from_le_bytes([200, 10, 10, 255])).unwrap();
        let mut tree = SparseTree::from_voxels(
            r,
            (8..16u32).flat_map(|x| {
                (8..16u32).flat_map(move |y| {
                    (8..12u32).map(move |z| {
                        (v(x, y, z), if z < 10 { red } else { 0u16 }) // mixed slab
                    })
                })
            }),
        );
        tree.promote_colors(|c, gid| promotion_color(&table, n, c, gid));
        assert!(tree.has_colors());
        assert_eq!(
            tree.color_at(v(9, 9, 9)),
            Some(u32::from_le_bytes([200, 10, 10, 255])),
            "palette voxel takes the table colour"
        );
        assert_eq!(
            tree.color_at(v(9, 9, 11)),
            Some(position_shade(v(9, 9, 11), n)),
            "global-0 voxel keeps its on-screen position shade"
        );

        // The promoted tree paints like any truecolor scene.
        let mut structure = SchoolBBuffer::from_sparse(&tree);
        let paint = BrushParams {
            tool: BrushTool::Paint,
            radius: 2,
            strength: 1.0,
            falloff: Falloff::Smooth,
            color: u32::from_le_bytes([10, 220, 40, 255]),
            invert: false,
        };
        let out = stroke_event(
            &mut tree,
            &mut structure,
            &paint,
            None,
            v(12, 12, 11),
            &mut StrokeState::default(),
        );
        assert!(out.changed > 0);
        assert_in_step(&tree, &structure);
    }

    /// The max-alpha mask: two overlapping paint stamps in one stroke apply the
    /// single strongest alpha, never a compounded one.
    #[test]
    fn stroke_mask_prevents_overlap_compounding() {
        let r = Resolution::new(32).expect("legal");
        let mut tree = SparseTree::from_voxels(
            r,
            (8..16u32).flat_map(|x| {
                (8..16u32)
                    .flat_map(move |y| (8..16u32).map(move |z| (VoxelCoord::new(x, y, z), 0u16)))
            }),
        );
        let occ = usize::try_from(tree.occupied_voxels()).unwrap();
        tree.install_colors(std::iter::repeat_n(u32::from_le_bytes([0, 0, 0, 255]), occ));
        let mut structure = SchoolBBuffer::from_sparse(&tree);

        let paint = BrushParams {
            tool: BrushTool::Paint,
            radius: 3,
            strength: 1.0,
            falloff: Falloff::Linear,
            color: u32::from_le_bytes([255, 255, 255, 255]),
            invert: false,
        };
        let target = VoxelCoord::new(12, 12, 12);
        let mut state = StrokeState::default();
        stroke_event(&mut tree, &mut structure, &paint, None, target, &mut state);
        let after_first = tree.color_at(target);
        stroke_event(
            &mut tree,
            &mut structure,
            &paint,
            Some(Stamp {
                center: target,
                pressure: 1.0,
            }),
            target,
            &mut state,
        );
        assert_eq!(
            tree.color_at(target),
            after_first,
            "re-stamp at equal alpha is a no-op"
        );
    }
}
