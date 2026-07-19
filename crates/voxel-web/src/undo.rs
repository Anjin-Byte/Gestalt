//! Per-stroke undo/redo (`docs/design/brush-editing/05`, Stage B): brick-
//! granular pre/post images keyed by the brick's Morton **code** — stable
//! across the leaf renumbering a topology edit causes — inside a byte-budgeted
//! ring. Images over inverse operations: a [`BrickImage`] is trivially correct
//! to capture and restore, and its byte-equality oracle is sharp.
//!
//! Capture happens at the single mutation seam ([`crate::edit::apply_stroke`]
//! journals every candidate brick *before* touching it), so no brush — present
//! or future — can mutate outside the journal. Everything here is pure and
//! natively testable; the GPU sync after a restore stays in the engine.

use std::collections::{HashMap, VecDeque};

use voxel_core::{BrickImage, SparseTree};

/// The history budget (locked decision): the ring holds at most this many
/// bytes of brick images, evicting oldest strokes first. Sized so a sculpting
/// session's local iteration loop (~50 worst-case strokes, hundreds of typical
/// ones) fits alongside the 3 GiB scene budget in the wasm32 address space —
/// the pin lives with the scene-budget pins in `scene_transfer.rs`.
pub(crate) const UNDO_BUDGET_BYTES: usize = 128 << 20;

/// A restore batch: per brick, its code and the image to restore (`None` =
/// the brick did not exist on that side and is removed).
pub(crate) type Restore = Vec<(u64, Option<BrickImage>)>;

/// One stroke's history record: per touched brick `(code, pre, post)`.
pub(crate) struct StrokeDelta {
    bricks: Vec<(u64, Option<BrickImage>, Option<BrickImage>)>,
    bytes: usize,
}

/// Collects the bricks a stroke touches, snapshotting each brick's pre-image
/// on **first** touch (later stamps of the same stroke re-touching the brick
/// cost one hash lookup). Post-images are taken at [`commit`](Self::commit).
#[derive(Default)]
pub(crate) struct StrokeJournal {
    pre: HashMap<u64, Option<BrickImage>>,
}

impl StrokeJournal {
    /// Journals brick `code` before a mutation: records its current state the
    /// first time this stroke sees it (`None` when the brick doesn't exist).
    pub(crate) fn capture(&mut self, tree: &SparseTree, code: u64) {
        self.pre
            .entry(code)
            .or_insert_with(|| tree.brick_image(code));
    }

    /// Ends the stroke: snapshots post-images, drops bricks that ended
    /// byte-identical (a stroke of misses records nothing), and returns the
    /// delta — `None` when nothing actually changed. Leaves the journal empty.
    pub(crate) fn commit(&mut self, tree: &SparseTree) -> Option<StrokeDelta> {
        let mut bricks: Vec<_> = self
            .pre
            .drain()
            .filter_map(|(code, pre)| {
                let post = tree.brick_image(code);
                (pre != post).then_some((code, pre, post))
            })
            .collect();
        if bricks.is_empty() {
            return None;
        }
        // Drain order is hash order; sort so deltas are deterministic.
        bricks.sort_unstable_by_key(|&(code, _, _)| code);
        let bytes = bricks
            .iter()
            .map(|(_, pre, post)| {
                // Code + option/vec headers, rounded; the images dominate.
                32 + pre.as_ref().map_or(0, BrickImage::bytes)
                    + post.as_ref().map_or(0, BrickImage::bytes)
            })
            .sum();
        Some(StrokeDelta { bricks, bytes })
    }

    /// Drops any captured state (scene install mid-stroke).
    pub(crate) fn clear(&mut self) {
        self.pre.clear();
    }
}

/// The byte-budgeted per-stroke history: an undo stack (`done`, oldest at the
/// front, evicted first) and a redo stack (`undone`, cleared by any new
/// stroke — standard semantics).
pub(crate) struct UndoRing {
    done: VecDeque<StrokeDelta>,
    undone: Vec<StrokeDelta>,
    /// Total image bytes across both stacks (an undo moves a delta between
    /// stacks without changing the total).
    bytes: usize,
    budget: usize,
}

impl Default for UndoRing {
    fn default() -> Self {
        Self::with_budget(UNDO_BUDGET_BYTES)
    }
}

impl UndoRing {
    /// A ring with an explicit budget — tests shrink it to drive eviction
    /// without megabytes of strokes. Production uses [`Default`].
    pub(crate) fn with_budget(budget: usize) -> Self {
        Self {
            done: VecDeque::new(),
            undone: Vec::new(),
            bytes: 0,
            budget,
        }
    }

    /// Records a committed stroke: clears the redo stack (a new stroke
    /// invalidates it), then evicts oldest strokes while over budget. A single
    /// stroke larger than the whole budget is kept alone — a history of one
    /// beats a history of none.
    pub(crate) fn push(&mut self, delta: StrokeDelta) {
        for dropped in self.undone.drain(..) {
            self.bytes -= dropped.bytes;
        }
        self.bytes += delta.bytes;
        self.done.push_back(delta);
        while self.bytes > self.budget && self.done.len() > 1 {
            let evicted = self.done.pop_front().expect("len > 1");
            self.bytes -= evicted.bytes;
        }
    }

    /// The most recent stroke's **pre** side, moving the stroke to the redo
    /// stack. `None` when there is nothing to undo.
    pub(crate) fn undo(&mut self) -> Option<Restore> {
        let delta = self.done.pop_back()?;
        let restore = delta
            .bricks
            .iter()
            .map(|(code, pre, _)| (*code, pre.clone()))
            .collect();
        self.undone.push(delta);
        Some(restore)
    }

    /// The most recently undone stroke's **post** side, moving it back onto
    /// the undo stack. `None` when there is nothing to redo.
    pub(crate) fn redo(&mut self) -> Option<Restore> {
        let delta = self.undone.pop()?;
        let restore = delta
            .bricks
            .iter()
            .map(|(code, _, post)| (*code, post.clone()))
            .collect();
        self.done.push_back(delta);
        Some(restore)
    }

    /// Strokes available to undo (drives the HUD button + depth display).
    pub(crate) fn undo_depth(&self) -> u32 {
        u32::try_from(self.done.len()).unwrap_or(u32::MAX)
    }

    /// Strokes available to redo.
    pub(crate) fn redo_depth(&self) -> u32 {
        u32::try_from(self.undone.len()).unwrap_or(u32::MAX)
    }

    /// Drops all history — a scene install (and, later, promotion) invalidates
    /// every image in the ring.
    pub(crate) fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
        self.bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec3;
    use voxel_brush::{BrushParams, BrushTool, Falloff, Stamp};
    use voxel_core::{Resolution, SchoolBBuffer, VoxelCoord};

    use super::*;
    use crate::edit::{StrokeEvent, StrokeState, apply_stroke};

    /// Equality up to page addresses: occupancy structure, node levels,
    /// materials and bounds byte-for-byte, colours per slot by content. Page
    /// addresses may legally differ after a restore (a page can be re-placed),
    /// so `leaf_color_page_words` is deliberately excluded — the oracle is
    /// patched-page *content*.
    fn assert_state_eq(a: &voxel_core::SparseTree, b: &voxel_core::SparseTree, at: &str) {
        let (sa, sb) = (SchoolBBuffer::from_sparse(a), SchoolBBuffer::from_sparse(b));
        assert_eq!(sa.nodes(), sb.nodes(), "{at}: nodes");
        assert_eq!(sa.leaves(), sb.leaves(), "{at}: leaves");
        assert_eq!(sa.leaf_mat_words(), sb.leaf_mat_words(), "{at}: materials");
        assert_eq!(
            sa.leaf_bounds_words(),
            sb.leaf_bounds_words(),
            "{at}: bounds"
        );
        assert_eq!(a.occupied_voxels(), b.occupied_voxels(), "{at}: voxels");
        assert_eq!(a.leaf_count(), b.leaf_count(), "{at}: leaf count");
        for i in 0..a.leaf_count() {
            assert_eq!(
                a.leaf_colors(i),
                b.leaf_colors(i),
                "{at}: colours, slot {i}"
            );
        }
    }

    /// A miniature engine: tree + structure + the undo machinery, driving real
    /// `apply_stroke` calls exactly as `Engine::brush`/`brush_end` do.
    struct Sim {
        tree: voxel_core::SparseTree,
        structure: SchoolBBuffer,
        ring: UndoRing,
        stroke: StrokeState,
    }

    impl Sim {
        /// A coloured 32³ block scene (8..24 × 8..24 × 8..16).
        fn colored() -> Self {
            let r = Resolution::new(32).expect("legal");
            let mut tree = voxel_core::SparseTree::from_voxels(
                r,
                (8..24u32).flat_map(|x| {
                    (8..24u32).flat_map(move |y| {
                        (8..16u32).map(move |z| (VoxelCoord::new(x, y, z), 0u16))
                    })
                }),
            );
            let occ = usize::try_from(tree.occupied_voxels()).unwrap();
            tree.install_colors((0..occ).map(|i| 0xFF00_0000 | u32::try_from(i % 0xFFFF).unwrap()));
            let structure = SchoolBBuffer::from_sparse(&tree);
            Self {
                tree,
                structure,
                ring: UndoRing::default(),
                stroke: StrokeState::default(),
            }
        }

        /// One full stroke: a stamp per point (bridging like the engine), then
        /// the reset + commit `brush_end` performs.
        fn stroke(&mut self, params: &BrushParams, points: &[VoxelCoord]) {
            let mut prev: Option<Stamp> = None;
            for &p in points {
                apply_stroke(
                    &mut self.tree,
                    &mut self.structure,
                    params,
                    prev,
                    StrokeEvent {
                        hit: p,
                        pressure: 1.0,
                        fallback_normal: DVec3::Z,
                    },
                    &mut self.stroke,
                );
                prev = Some(Stamp {
                    center: p,
                    pressure: 1.0,
                });
            }
            self.stroke.mask.clear();
            self.stroke.anchor = None;
            if let Some(delta) = self.stroke.journal.commit(&self.tree) {
                self.ring.push(delta);
            }
        }

        fn undo(&mut self) -> bool {
            self.time_travel(true)
        }

        fn redo(&mut self) -> bool {
            self.time_travel(false)
        }

        fn time_travel(&mut self, back: bool) -> bool {
            let restore = if back {
                self.ring.undo()
            } else {
                self.ring.redo()
            };
            let Some(restore) = restore else {
                return false;
            };
            self.tree.replace_bricks(restore);
            self.structure = SchoolBBuffer::from_sparse(&self.tree);
            true
        }
    }

    fn tool(t: BrushTool, radius: u32, color: u32) -> BrushParams {
        BrushParams {
            tool: t,
            radius,
            strength: 1.0,
            falloff: Falloff::Smooth,
            color,
            invert: false,
        }
    }

    /// The script the byte-equality oracles run: a topology-heavy mix — draw
    /// into empty space (bricks created), a paint drag, a carving erase (bricks
    /// destroyed), and a draw that recreates ground the erase cleared.
    fn scripted_strokes(sim: &mut Sim) -> Vec<voxel_core::SparseTree> {
        let v = VoxelCoord::new;
        let strokes: Vec<(BrushParams, Vec<VoxelCoord>)> = vec![
            (
                tool(BrushTool::Draw, 2, 0xFFCC_2211),
                vec![v(27, 27, 27), v(27, 27, 22)],
            ),
            (
                tool(BrushTool::Paint, 4, 0xFF11_EE22),
                vec![v(10, 10, 12), v(20, 12, 12)],
            ),
            // The sculpt set (Stage C): the journal seam must hold under every
            // tool, so undo-all crosses all seven.
            (
                tool(BrushTool::Clay, 3, 0xFF55_1188),
                vec![v(12, 12, 15), v(16, 12, 15)],
            ),
            (tool(BrushTool::Smooth, 3, 0), vec![v(14, 14, 15)]),
            (
                tool(BrushTool::Flatten, 3, 0),
                vec![v(18, 18, 15), v(20, 18, 15)],
            ),
            (tool(BrushTool::Inflate, 3, 0), vec![v(10, 20, 15)]),
            (
                tool(BrushTool::Erase, 3, 0),
                vec![v(12, 12, 12), v(12, 20, 12)],
            ),
            (tool(BrushTool::Draw, 2, 0xFF33_44AA), vec![v(12, 12, 12)]),
        ];
        // Record a checkpoint only for strokes that actually pushed a delta,
        // so the state walk stays 1:1 with the ring even if a sculpt stroke
        // happens to be a no-op on this scene.
        let mut states = Vec::new();
        for (params, points) in strokes {
            let before = sim.ring.undo_depth();
            sim.stroke(&params, &points);
            if sim.ring.undo_depth() > before {
                states.push(sim.tree.clone());
            }
        }
        states
    }

    /// THE Stage-B oracle: undo-all walks back through every intermediate
    /// state byte-exactly (up to page addresses), redo-all walks forward the
    /// same way — across topology-changing strokes, which renumber every leaf
    /// index and so pin the Morton-code keying. The script covers all seven
    /// tools.
    #[test]
    fn undo_all_then_redo_all_restore_every_state() {
        let mut sim = Sim::colored();
        let initial = sim.tree.clone();
        let states = scripted_strokes(&mut sim);
        let depth = sim.ring.undo_depth();
        assert_eq!(depth as usize, states.len());
        assert!(
            depth >= 6,
            "the tool sweep must be substantive, got {depth}"
        );
        assert_eq!(sim.ring.redo_depth(), 0);

        // Undo all: after popping stroke i we must sit at states[i-1].
        for i in (0..states.len()).rev() {
            assert!(sim.undo(), "undo {i}");
            let expect = if i == 0 { &initial } else { &states[i - 1] };
            assert_state_eq(&sim.tree, expect, &format!("undo to state {i}"));
        }
        assert!(!sim.undo(), "history exhausted");
        assert_eq!(sim.ring.redo_depth(), depth);

        // Redo all: forward through the same states.
        for (i, state) in states.iter().enumerate() {
            assert!(sim.redo(), "redo {i}");
            assert_state_eq(&sim.tree, state, &format!("redo to state {i}"));
        }
        assert!(!sim.redo(), "future exhausted");
    }

    /// Standard semantics: a new stroke clears the redo stack; interleaved
    /// undo/redo around it stays byte-exact.
    #[test]
    fn a_new_stroke_clears_redo_and_interleavings_stay_exact() {
        let v = VoxelCoord::new;
        let mut sim = Sim::colored();
        let initial = sim.tree.clone();
        sim.stroke(&tool(BrushTool::Draw, 2, 0xFF11_1111), &[v(27, 27, 27)]);
        let after_s1 = sim.tree.clone();
        sim.stroke(&tool(BrushTool::Erase, 2, 0), &[v(12, 12, 12)]);

        assert!(sim.undo());
        assert_state_eq(&sim.tree, &after_s1, "back to s1");
        assert_eq!(sim.ring.redo_depth(), 1);

        // A new stroke invalidates the undone erase.
        sim.stroke(&tool(BrushTool::Paint, 3, 0xFFEE_9900), &[v(10, 10, 12)]);
        let after_s3 = sim.tree.clone();
        assert_eq!(sim.ring.redo_depth(), 0);
        assert!(!sim.redo(), "redo cleared by the new stroke");

        assert!(sim.undo() && sim.undo());
        assert_state_eq(&sim.tree, &initial, "unwound to initial");
        assert!(sim.redo() && sim.redo());
        assert_state_eq(&sim.tree, &after_s3, "rewound to the new timeline");
    }

    /// A stroke that changes nothing (erasing empty space) records nothing —
    /// the journal drops bricks whose pre and post images are identical.
    #[test]
    fn a_no_op_stroke_records_no_history() {
        let v = VoxelCoord::new;
        let mut sim = Sim::colored();
        sim.stroke(&tool(BrushTool::Erase, 2, 0), &[v(28, 28, 28)]);
        assert_eq!(sim.ring.undo_depth(), 0);
        // Painting outside occupancy is likewise a no-op.
        sim.stroke(&tool(BrushTool::Paint, 2, 0xFF00_FF00), &[v(28, 28, 28)]);
        assert_eq!(sim.ring.undo_depth(), 0);
    }

    /// Eviction: over budget, the oldest strokes drop; depth tells the truth;
    /// the surviving history still restores exactly — landing at the state
    /// *after* the evicted strokes, not the initial one.
    #[test]
    fn over_budget_evicts_oldest_and_the_rest_still_restore() {
        let v = VoxelCoord::new;
        let mut sim = Sim::colored();
        // Budget two-ish strokes: each single-voxel draw delta is ~hundreds of
        // bytes; 1 KiB keeps a couple and evicts the rest.
        sim.ring = UndoRing::with_budget(1024);
        let mut states = Vec::new();
        for i in 0..6u32 {
            sim.stroke(
                &tool(BrushTool::Draw, 0, 0xFF00_0000 | i),
                &[v(25 + (i % 3) * 2, 25, 25 + (i / 3) * 2)],
            );
            states.push(sim.tree.clone());
        }
        let depth = sim.ring.undo_depth() as usize;
        assert!(
            depth < 6,
            "budget must have evicted something (depth {depth})"
        );
        assert!(depth >= 1, "a history of at least one survives");

        // Undo everything that survives: we land at the state after the
        // evicted prefix (strokes 0..6-depth), byte-exactly.
        for _ in 0..depth {
            assert!(sim.undo());
        }
        assert!(!sim.undo());
        assert_state_eq(
            &sim.tree,
            &states[6 - depth - 1],
            "the eviction horizon state",
        );
        // And the future replays exactly.
        for _ in 0..depth {
            assert!(sim.redo());
        }
        assert_state_eq(&sim.tree, &states[5], "back to the final state");
    }

    /// A single stroke larger than the whole budget is kept alone (a history
    /// of one beats a history of none).
    #[test]
    fn one_oversized_stroke_survives_alone() {
        let v = VoxelCoord::new;
        let mut sim = Sim::colored();
        sim.ring = UndoRing::with_budget(64); // smaller than any real delta
        let initial = sim.tree.clone();
        sim.stroke(&tool(BrushTool::Draw, 3, 0xFF12_3456), &[v(27, 27, 27)]);
        assert_eq!(sim.ring.undo_depth(), 1);
        assert!(sim.undo());
        assert_state_eq(&sim.tree, &initial, "the oversized stroke undoes");
    }
}
