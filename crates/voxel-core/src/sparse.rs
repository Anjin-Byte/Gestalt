//! P3: the sparse `4³` node hierarchy over Morton-ordered leaves.
//!
//! This is `idea.md` §11.3 — sparse leaves addressed by `popcount`-rank,
//! built by the §6.4 pipeline (enumerate → Morton-sort → bottom-up `4³`
//! OR-reduce). Only non-empty cells are stored. The traversal is an N-level
//! hierarchical Amanatides–Woo: a recursive descent that walks each level's
//! `4³` (or `8³` at the leaf) cells and recurses into occupied children — the
//! clean `f64` reference form of `idea.md` §7. P4 re-serializes this into the
//! School-B single buffer and adds the `f32` GPU mirror; the traversal logic is
//! identical.
//!
//! Per-level node arrays (a School-A layout) are used here; the §10 gate (P3.5)
//! decides whether the School-B interleaving is worth it (review R5).

use crate::color_pool::{self, ColorPool, PAGE_NONE};
use crate::layout::{Cell, NodeLayout, TraversalStats};
use crate::leaf::LeafBrick;
use crate::node::{self, GpuNode};
use crate::oracle::Hit;
use crate::palette::{LEAF_VOXELS, P_CAP};
use crate::ray::Ray;
use crate::{OccupancyField, Resolution, VoxelCoord};

/// A sparse hierarchy: Morton-ordered leaf bricks and, per internal level, a
/// packed array of `4³` nodes addressed by `popcount`-rank.
#[derive(Debug, Clone)]
pub struct SparseTree {
    resolution: Resolution,
    /// Internal nodes by traversal level. `nodes[L]` holds level-`L` nodes for
    /// `L ∈ 2..=k+1`; indices `0` and `1` are unused (the leaf array is "level
    /// 1"). The root is `nodes[k+1][0]` (or `leaves[0]` when `k = 0`).
    nodes: Vec<Vec<GpuNode>>,
    /// Non-empty leaf bricks, sorted by brick Morton code.
    leaves: Vec<LeafBrick>,
    /// Brick Morton codes, parallel to and in the same order as `leaves`
    /// (ascending). Retained to support incremental edits ([`SparseTree::set_voxel`]):
    /// a topology change binary-searches and splices this list, then rebuilds the
    /// node levels from it — skipping the `O(n³)` occupancy scan.
    codes: Vec<u64>,
    /// Monotonic counter bumped on every topology change (a brick appearing or
    /// disappearing). A [`SchoolBBuffer`] records this at `from_sparse` time and
    /// asserts it is unchanged before an in-place [`patch_leaf`](crate::SchoolBBuffer::patch_leaf):
    /// a topology edit renumbers leaf indices, so a stale patch would corrupt the
    /// buffer silently — this turns that into a loud panic.
    topo_gen: u64,
    /// Per-voxel materials: one global material id per voxel in intra-brick
    /// Morton order (`0` = the reserved default / MISSING sentinel). The GPU's
    /// packed per-leaf palette is *derived* from this at upload time (so the
    /// palette is always minimal — no CPU palette/GC). See [`MaterialStore`]
    /// for why the never-coloured case stores nothing.
    materials: MaterialStore,
    /// Per-voxel **editable** truecolor (brush-editing Stage A1). `None` for
    /// every non-truecolor scene (fixtures, palette, occupancy-only), which pay
    /// nothing. Installed via [`install_colors`](Self::install_colors) and edited
    /// in place; splices in lockstep with `leaves`/`codes`. Independent of
    /// `materials` — a scene is palette-coloured *or* truecolor, never mixing the
    /// two stores' reads.
    colors: ColorStore,
}

/// The tree's per-voxel material storage. Most scenes never colour a voxel
/// (fixtures, noise, plain occupancy): [`Uniform`](Self::Uniform) represents
/// "every occupied voxel is global-0" with **no storage at all** — a dense
/// grid costs ~1 KiB per leaf, which at high resolution is the difference
/// between megabytes and gigabytes of resident memory (the wasm32 build lives
/// in a 4 GiB address space that never shrinks). The store densifies lazily on
/// the first non-zero write and never converts back.
#[derive(Debug, Clone)]
#[allow(clippy::vec_box)] // boxed so a topology splice shifts 8-byte pointers, not 1 KiB grids
enum MaterialStore {
    /// Every occupied voxel reads global-0; nothing is stored.
    Uniform,
    /// One per-voxel grid per leaf, index-parallel with `leaves`/`codes`,
    /// spliced in lockstep on every topology edit.
    Dense(Vec<Box<[u16; LEAF_VOXELS]>>),
}

/// The all-zero grid [`MaterialStore::Uniform`] leaves read through.
static UNIFORM_MATERIALS: [u16; LEAF_VOXELS] = [0u16; LEAF_VOXELS];

impl MaterialStore {
    /// Leaf `idx`'s grid (the shared zero grid while `Uniform`).
    fn of(&self, idx: usize) -> &[u16; LEAF_VOXELS] {
        match self {
            MaterialStore::Uniform => &UNIFORM_MATERIALS,
            MaterialStore::Dense(grids) => &grids[idx],
        }
    }

    /// Topology splice: drops leaf `idx`'s grid (no-op while `Uniform`).
    fn remove(&mut self, idx: usize) {
        if let MaterialStore::Dense(grids) = self {
            grids.remove(idx);
        }
    }

    /// Topology splice: inserts a fresh all-default grid at `idx` (no-op while
    /// `Uniform` — a default grid is exactly what `Uniform` already means).
    fn insert_default(&mut self, idx: usize) {
        if let MaterialStore::Dense(grids) = self {
            grids.insert(idx, Box::new([0u16; LEAF_VOXELS]));
        }
    }

    /// The dense grids, materializing them on first need (`leaf_count` zero
    /// grids). The one-way door out of `Uniform`.
    #[allow(clippy::vec_box)] // the splice-friendly box-per-leaf shape, as stored
    fn densify(&mut self, leaf_count: usize) -> &mut Vec<Box<[u16; LEAF_VOXELS]>> {
        if matches!(self, MaterialStore::Uniform) {
            *self = MaterialStore::Dense(
                (0..leaf_count)
                    .map(|_| Box::new([0u16; LEAF_VOXELS]))
                    .collect(),
            );
        }
        let MaterialStore::Dense(grids) = self else {
            unreachable!("densified above");
        };
        grids
    }
}

/// The colour written to a voxel newly set through the occupancy-only path
/// ([`SparseTree::set_voxel`]) on a coloured tree: opaque black. Occupancy edits
/// must keep every leaf's colour array the same length as its occupancy, so a
/// bare set still writes *a* colour; a caller wanting a specific one uses
/// [`SparseTree::set_voxel_colored`].
const DEFAULT_COLOR: u32 = 0xFF00_0000; // [0, 0, 0, 255] little-endian (R low)

/// Whether a packed sRGB RGBA8 colour (R in the low byte) is semi-transparent —
/// alpha (the high byte) below 255. Drives the per-leaf transparency tracking.
fn is_transparent(rgba: u32) -> bool {
    (rgba >> 24) < 255
}

/// The tree's **editable** per-voxel truecolor store (brush-editing Stage A1,
/// `docs/design/brush-editing/02`). Only truecolor scenes carry one; every other
/// scene (fixtures, palette, occupancy-only) stays [`None`](Self::None) and pays
/// nothing. Unlike the build-once bake ([`SchoolBBuffer::assemble_leaf_color`]),
/// this is authoritative and splices in lockstep with `leaves`/`codes`, so an
/// occupancy edit keeps colours consistent without a full re-bake.
///
/// [`SchoolBBuffer::assemble_leaf_color`]: crate::SchoolBBuffer::assemble_leaf_color
#[derive(Debug, Clone)]
enum ColorStore {
    /// No per-voxel colour.
    None,
    /// One page per leaf, index-parallel with `leaves`/`codes`. Boxed so the
    /// enum stays pointer-sized — the common `None` scenes pay nothing.
    PerVoxel(Box<ColorLeaves>),
}

/// One leaf's editable colours: rank-order sRGB RGBA8 words plus the pool page
/// they occupy. Spliced in lockstep with `leaves`, exactly like a
/// [`MaterialStore::Dense`] grid.
#[derive(Debug, Clone)]
struct ColorLeaf {
    /// Rank-order colours; `data.len() == leaf.count_occupied()` at all times.
    data: Vec<u32>,
    /// Pool entry offset of this leaf's page ([`PAGE_NONE`] only transiently).
    page: u32,
    /// Page capacity in entries (`color_pool::class_for(data.len())`).
    class: u32,
    /// Whether any stored colour is semi-transparent (alpha < 255).
    transparent: bool,
}

/// The per-leaf colour pages plus the pool allocator that places them. The page
/// offsets are GPU addresses (Stage A2 uploads the pool), stored here so a
/// topology edit renumbers a 4 B/leaf page table rather than rebuilding the
/// pool — see `docs/design/brush-editing/02`.
#[derive(Debug, Clone)]
struct ColorLeaves {
    /// Index-parallel with [`SparseTree::leaves`].
    leaves: Vec<ColorLeaf>,
    /// Page allocator (per-chunk watermarks + per-class free lists).
    pool: ColorPool,
    /// Count of leaves with at least one transparent voxel (keeps the scene's
    /// `has_transparency` current under edits without a rescan).
    transparent_leaves: u32,
}

impl ColorLeaves {
    /// Re-derives leaf `idx`'s page capacity from its current occupancy,
    /// reallocating (free old + alloc new) only when the size class changes.
    /// Keeps `class == class_for(occupancy)` so the pool waste bound holds.
    fn reclass(&mut self, idx: usize) {
        let want = color_pool::class_for(u32::try_from(self.leaves[idx].data.len()).unwrap_or(0));
        let cur = self.leaves[idx].class;
        if want == cur {
            return;
        }
        let old_page = self.leaves[idx].page;
        if old_page != PAGE_NONE && cur > 0 {
            self.pool.free(old_page, cur);
        }
        let new_page = if want > 0 {
            self.pool.alloc(want)
        } else {
            PAGE_NONE
        };
        self.leaves[idx].page = new_page;
        self.leaves[idx].class = want;
    }

    /// Recomputes leaf `idx`'s transparency flag and keeps the scene count in
    /// step. O(occupancy); called after any colour write to that leaf.
    fn refresh_transparent(&mut self, idx: usize) {
        let now = self.leaves[idx].data.iter().copied().any(is_transparent);
        let was = self.leaves[idx].transparent;
        if now != was {
            self.leaves[idx].transparent = now;
            if now {
                self.transparent_leaves += 1;
            } else {
                self.transparent_leaves -= 1;
            }
        }
    }

    /// Splices a fresh single-voxel colour leaf in at slot `at` (a new brick).
    fn insert_leaf(&mut self, at: usize, color: u32) {
        self.insert_leaf_data(at, vec![color]);
    }

    /// Splices a colour leaf with the given rank-order `data` in at slot `at`
    /// (a brick restored whole — [`SparseTree::replace_bricks`]). Allocates a
    /// pool page for its size class.
    fn insert_leaf_data(&mut self, at: usize, data: Vec<u32>) {
        let class = color_pool::class_for(u32::try_from(data.len()).unwrap_or(0));
        let page = if class > 0 {
            self.pool.alloc(class)
        } else {
            PAGE_NONE
        };
        let transparent = data.iter().copied().any(is_transparent);
        if transparent {
            self.transparent_leaves += 1;
        }
        self.leaves.insert(
            at,
            ColorLeaf {
                data,
                page,
                class,
                transparent,
            },
        );
    }

    /// Replaces leaf `idx`'s rank-order colours wholesale (a brick restored in
    /// place), then reclasses its page and refreshes transparency.
    fn replace_leaf_data(&mut self, idx: usize, data: Vec<u32>) {
        self.leaves[idx].data = data;
        self.reclass(idx);
        self.refresh_transparent(idx);
    }

    /// Drops leaf `at`'s colour page (a brick disappearing), returning its page
    /// to the pool.
    fn remove_leaf(&mut self, at: usize) {
        let cl = self.leaves.remove(at);
        if cl.page != PAGE_NONE && cl.class > 0 {
            self.pool.free(cl.page, cl.class);
        }
        if cl.transparent {
            self.transparent_leaves -= 1;
        }
    }

    /// Inserts a colour at `rank` within leaf `idx` (a voxel newly set), then
    /// reclasses and refreshes transparency.
    fn insert_voxel(&mut self, idx: usize, rank: usize, color: u32) {
        self.leaves[idx].data.insert(rank, color);
        self.reclass(idx);
        self.refresh_transparent(idx);
    }

    /// Removes the colour at `rank` within leaf `idx` (a voxel cleared), then
    /// reclasses and refreshes transparency.
    fn remove_voxel(&mut self, idx: usize, rank: usize) {
        self.leaves[idx].data.remove(rank);
        self.reclass(idx);
        self.refresh_transparent(idx);
    }

    /// Recolours the voxel at `rank` within leaf `idx`. Returns whether the
    /// colour actually changed (a same-colour write is a no-op).
    fn recolor(&mut self, idx: usize, rank: usize, color: u32) -> bool {
        if self.leaves[idx].data[rank] == color {
            return false;
        }
        self.leaves[idx].data[rank] = color;
        self.refresh_transparent(idx);
        true
    }
}

/// A borrowed read view of a coloured tree's pool state, for the GPU upload
/// (Stage A2) and the School-B page-table derivation. Yields, per leaf slot, the
/// page offset, its class capacity, and the rank-order colours, plus the total
/// pool extent to allocate.
pub struct ColorPages<'a> {
    store: &'a ColorLeaves,
}

impl ColorPages<'_> {
    /// Number of coloured leaf slots (index-parallel with the tree's leaves).
    #[must_use]
    pub fn len(&self) -> usize {
        self.store.leaves.len()
    }

    /// Whether there are no coloured leaves (an empty scene).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.store.leaves.is_empty()
    }

    /// Leaf `slot`'s page entry offset in the pool.
    #[must_use]
    pub fn page_of(&self, slot: usize) -> u32 {
        self.store.leaves[slot].page
    }

    /// Leaf `slot`'s page capacity in entries.
    #[must_use]
    pub fn class_of(&self, slot: usize) -> u32 {
        self.store.leaves[slot].class
    }

    /// Leaf `slot`'s rank-order colours (length = the leaf's occupied count).
    #[must_use]
    pub fn colors_of(&self, slot: usize) -> &[u32] {
        &self.store.leaves[slot].data
    }

    /// Whether leaf `slot` carries a semi-transparent voxel (alpha < 255) — the
    /// source of its GPU `leaf_bounds` transparency bit.
    #[must_use]
    pub fn leaf_transparent(&self, slot: usize) -> bool {
        self.store.leaves[slot].transparent
    }

    /// The pool's high-water extent in entries — the size the GPU pool must span.
    #[must_use]
    pub fn total_entries(&self) -> u64 {
        self.store.pool.watermark()
    }

    /// The pool's chunk size in entries. The GPU maps the flat entry space onto
    /// chunk buffers at exactly this granularity, so a page (which never straddles
    /// a chunk) is addressed by `offset / chunk_entries`. The GPU renderer reads
    /// this as its `PER_CHUNK`, keeping CPU page placement and GPU chunk-select in
    /// lockstep.
    #[must_use]
    pub fn chunk_entries(&self) -> u64 {
        self.store.pool.chunk_entries()
    }

    /// Whether any leaf carries a semi-transparent voxel.
    #[must_use]
    pub fn has_transparency(&self) -> bool {
        self.store.transparent_leaves > 0
    }
}

/// What an edit ([`SparseTree::set_voxel`]) did to the structure — and therefore
/// how little a GPU adapter must re-upload to stay in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    /// The voxel already had the requested state; nothing changed.
    Unchanged,
    /// Exactly one leaf brick changed *in place* (the given leaf index): relative
    /// to the immediately-prior tree state, the Morton order, node masks, and all
    /// indices are unchanged, so the adapter need only re-upload that one leaf's
    /// words and bounds. (Indices are *not* stable across an intervening
    /// [`Topology`](Self::Topology) edit — see [`SchoolBBuffer::patch_leaf`].)
    ///
    /// [`SchoolBBuffer::patch_leaf`]: crate::SchoolBBuffer::patch_leaf
    Leaf(u32),
    /// A brick appeared or disappeared: the leaf array and node levels were
    /// rebuilt (no scan). Leaf indices have shifted; the adapter must
    /// re-serialize and re-upload the structure.
    Topology,
    /// A leaf's per-voxel material changed in place (no occupancy topology
    /// change): only that leaf's material slot need re-upload. `spilled` means
    /// the leaf now has more than `P_CAP` distinct occupied materials, so it no
    /// longer fits the inline palette and must ride the full reupload instead of
    /// an O(1) slot patch (treated as topology-class — the generation is bumped).
    Material {
        /// The affected leaf index (index-parallel with the GPU `leaf_mat` slot).
        leaf: u32,
        /// The leaf exceeded `P_CAP` distinct materials and now spills.
        spilled: bool,
    },
    /// A leaf's per-voxel truecolor changed in place (no occupancy topology
    /// change): only that leaf's colour page need re-upload, not its occupancy
    /// words. Returned by [`SparseTree::set_color`] (and by
    /// [`set_voxel_colored`](SparseTree::set_voxel_colored) when it only recolours
    /// an already-occupied voxel). Occupancy edits on a coloured tree still return
    /// [`Leaf`](Self::Leaf)/[`Topology`](Self::Topology); the caller re-uploads
    /// the colour page alongside on those, keyed by the leaf index.
    Color {
        /// The affected leaf index (index-parallel with the GPU colour page table).
        leaf: u32,
    },
}

/// A full snapshot of one `8³` brick — occupancy, materials, colours — keyed
/// externally by the brick's Morton **code** (stable across topology
/// renumbering, unlike leaf indices). The unit of the brush undo system's
/// pre/post images (`docs/design/brush-editing/05`): captured with
/// [`SparseTree::brick_image`], restored with [`SparseTree::replace_bricks`].
///
/// `materials: None` means the brick reads all-default (global-0) — both the
/// storage-free uniform material form and a dropped all-zero grid capture as
/// `None`. `colors: None` means the tree carried no truecolor store at
/// capture time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrickImage {
    occ: LeafBrick,
    materials: Option<Box<[u16; LEAF_VOXELS]>>,
    colors: Option<Vec<u32>>,
}

impl BrickImage {
    /// The snapshot's heap footprint in bytes (occupancy words + any material
    /// grid + any colour payload) — the currency of the undo ring's budget.
    #[must_use]
    pub fn bytes(&self) -> usize {
        64 + self.materials.as_ref().map_or(0, |_| 2 * LEAF_VOXELS)
            + self.colors.as_ref().map_or(0, |c| 4 * c.len())
    }
}

/// The voxel coordinates inside a solid sphere of `radius` voxels centred on
/// `center` (Euclidean membership: `dx² + dy² + dz² ≤ radius²`). `radius = 0`
/// yields just the centre voxel.
///
/// Coordinates that would fall below the grid origin are omitted; the upper
/// bound is left to [`SparseTree::set_voxel`], which treats out-of-bounds as
/// [`Edit::Unchanged`]. This is pure geometry shared by the viewer's edit brush
/// and the edit benchmarks, so both stamp the identical voxel set.
#[must_use]
pub fn brush_voxels(center: VoxelCoord, radius: u32) -> Vec<VoxelCoord> {
    let r = i64::from(radius);
    let r2 = r * r;
    let mut out = Vec::new();
    for dz in -r..=r {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy + dz * dz > r2 {
                    continue;
                }
                let (x, y, z) = (
                    i64::from(center.x) + dx,
                    i64::from(center.y) + dy,
                    i64::from(center.z) + dz,
                );
                if let (Ok(x), Ok(y), Ok(z)) =
                    (u32::try_from(x), u32::try_from(y), u32::try_from(z))
                {
                    out.push(VoxelCoord::new(x, y, z));
                }
            }
        }
    }
    out
}

/// Groups a sorted list of child Morton codes into `4³` parent nodes.
///
/// Returns the parent nodes (each with its child mask and the base index of its
/// children in the input array) and the parents' own Morton codes, ready to be
/// grouped again one level up.
fn build_parents(child_codes: &[u64]) -> (Vec<GpuNode>, Vec<u64>) {
    let mut nodes = Vec::new();
    let mut parent_codes = Vec::new();
    let mut i = 0;
    while i < child_codes.len() {
        let parent = child_codes[i] >> 6;
        let child_base = u32::try_from(i).expect("stored child count exceeds u32::MAX");
        let mut mask = 0u64;
        while i < child_codes.len() && (child_codes[i] >> 6) == parent {
            mask |= 1u64 << (child_codes[i] & 63);
            i += 1;
        }
        nodes.push(GpuNode::new(mask, child_base));
        parent_codes.push(parent);
    }
    (nodes, parent_codes)
}

/// Builds the internal node levels `2..=k+1` bottom-up from the (ascending)
/// leaf-brick Morton codes by repeatedly OR-reducing `4³` groups. `nodes[L]`
/// holds level-`L` nodes; indices `0`/`1` are unused. Shared by [`SparseTree::build`]
/// (after the scan) and [`SparseTree::set_voxel`] (after a topology splice) — the
/// latter is why it is `O(bricks)` and scan-free.
fn build_levels(k: u32, leaf_codes: &[u64]) -> Vec<Vec<GpuNode>> {
    let mut nodes: Vec<Vec<GpuNode>> = vec![Vec::new(); (k + 2) as usize];
    if leaf_codes.is_empty() {
        return nodes;
    }
    let mut codes = leaf_codes.to_vec();
    for level in 2..=(k + 1) {
        let (parents, parent_codes) = build_parents(&codes);
        nodes[level as usize] = parents;
        codes = parent_codes;
    }
    nodes
}

/// Narrows a leaf-array index to the `u32` used by [`Edit::Leaf`] and the GPU
/// buffers (leaf counts are bounded well below `u32::MAX` by the build).
fn leaf_index(idx: usize) -> u32 {
    u32::try_from(idx).expect("stored leaf count exceeds u32::MAX")
}

/// Whether a leaf has more than `P_CAP` distinct materials among its **occupied**
/// voxels — the spill condition. Only occupied voxels matter (the GPU palette is
/// built from them); empty voxels' material is irrelevant. Early-exits once the
/// cap is exceeded, so it is `O(512)` worst case with a tiny (≤17) linear scan.
fn leaf_spills(leaf: &LeafBrick, materials: &[u16; LEAF_VOXELS]) -> bool {
    let mut seen: Vec<u16> = Vec::with_capacity(P_CAP as usize + 1);
    for z in 0..8u32 {
        for y in 0..8u32 {
            for x in 0..8u32 {
                if leaf.get_local(x, y, z) {
                    let gid = materials[crate::morton::encode_brick(x, y, z) as usize];
                    if !seen.contains(&gid) {
                        seen.push(gid);
                        if seen.len() > P_CAP as usize {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// Enumerates occupied bricks in the z-slab `[bz_lo, bz_hi)` — the parallel
/// unit of [`SparseTree::build`]'s scan.
fn enumerate_slab<F: OccupancyField>(
    field: &F,
    bpa: u32,
    bz_lo: u32,
    bz_hi: u32,
) -> Vec<(u64, LeafBrick)> {
    let mut out = Vec::new();
    for bz in bz_lo..bz_hi {
        for by in 0..bpa {
            for bx in 0..bpa {
                let mut leaf = LeafBrick::EMPTY;
                for lz in 0..8 {
                    for ly in 0..8 {
                        for lx in 0..8 {
                            let c = VoxelCoord::new(bx * 8 + lx, by * 8 + ly, bz * 8 + lz);
                            if field.is_occupied(c) {
                                leaf.set_local(lx, ly, lz);
                            }
                        }
                    }
                }
                if !leaf.is_empty() {
                    out.push((crate::morton::encode(bx, by, bz), leaf));
                }
            }
        }
    }
    out
}

impl SparseTree {
    /// Builds the sparse hierarchy from an occupancy field (`idea.md` §6.4
    /// steps 1–4).
    ///
    /// # Examples
    /// ```
    /// use voxel_core::{Resolution, SparseTree, VoxelCoord};
    /// use voxel_core::fixtures::Solid;
    ///
    /// let tree = SparseTree::build(&Solid { resolution: Resolution::new(8).unwrap() });
    /// assert_eq!(tree.leaf_count(), 1); // an 8³ solid is a single full brick
    /// assert!(tree.is_occupied(VoxelCoord::new(0, 0, 0)));
    /// ```
    #[must_use]
    pub fn build<F: OccupancyField + Sync>(field: &F) -> Self {
        let resolution = field.resolution();
        let bpa = resolution.voxels_per_axis() / 8;

        // 1. Enumerate occupied bricks (Morton code + 512-bit leaf). This scan
        //    dominates build time at high resolution, so the z-slabs are split
        //    across threads (scoped, so `field` can be borrowed).
        let threads: u32 = std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .ok()
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(1)
            .clamp(1, bpa.max(1));
        // Spawn-free when there is nothing to split: `wasm32` has no runtime
        // threads (this crate's charter promises wasm32), and it reports
        // `available_parallelism` as an error, landing here with `threads == 1`.
        let bricks: Vec<(u64, LeafBrick)> = if threads == 1 {
            enumerate_slab(field, bpa, 0, bpa)
        } else {
            std::thread::scope(|scope| {
                let chunk = bpa.div_ceil(threads);
                let handles: Vec<_> = (0..threads)
                    .map(|t| {
                        let lo = t * chunk;
                        let hi = ((t + 1) * chunk).min(bpa);
                        scope.spawn(move || enumerate_slab(field, bpa, lo, hi))
                    })
                    .collect();
                handles
                    .into_iter()
                    .flat_map(|h| h.join().expect("brick-enumeration thread panicked"))
                    .collect()
            })
        };

        // 2–4. Sort the occupied bricks and build the internal levels.
        Self::from_bricks(resolution, bricks)
    }

    /// Assembles the sparse hierarchy from already-enumerated occupied bricks —
    /// `(Morton code, leaf)` pairs in any order, where the code is
    /// [`morton::encode`](crate::morton::encode)`(bx, by, bz)` of the brick. This
    /// is steps 2–4 of [`build`](Self::build) for callers that enumerated the
    /// occupancy themselves (e.g. a GPU generator that evaluated the field in
    /// parallel and read back only the non-empty bricks), skipping the per-voxel
    /// CPU scan entirely.
    #[must_use]
    pub fn from_bricks(resolution: Resolution, mut bricks: Vec<(u64, LeafBrick)>) -> Self {
        let k = resolution.internal_levels();
        // Sort by Morton code (codes are unique, so the order is total).
        bricks.sort_unstable_by_key(|(code, _)| *code);
        let leaves: Vec<LeafBrick> = bricks.iter().map(|(_, leaf)| *leaf).collect();
        let codes: Vec<u64> = bricks.into_iter().map(|(code, _)| code).collect();

        // Build internal levels 2..=k+1 bottom-up by OR-reducing 4³ groups.
        let nodes = build_levels(k, &codes);

        Self {
            resolution,
            nodes,
            leaves,
            codes,
            // Materials start as the storage-free all-0 sentinel; a builder
            // with material data (the voxelizer's owner→material) densifies
            // via `fill_materials` after construction.
            materials: MaterialStore::Uniform,
            // Truecolor is opt-in via `install_colors` (the deserialize seam);
            // a freshly-built tree carries no editable colour store.
            colors: ColorStore::None,
            topo_gen: 0,
        }
    }

    /// Reassembles a tree from its own decomposed parts — parallel per-leaf
    /// arrays of Morton codes, occupancy bricks, and material grids, as a
    /// serializer (e.g. a cross-worker scene transfer) captured them. The
    /// internal levels are rebuilt from the codes; the parts are **not**
    /// reinterpreted, so this is the exact inverse of walking
    /// [`leaf_origin`](Self::leaf_origin) / leaf bricks /
    /// [`leaf_materials`](Self::leaf_materials) in slot order.
    ///
    /// `codes` MUST be strictly ascending (the canonical leaf order every
    /// builder produces — a serializer walking slots in order preserves it)
    /// and the parallel slices MUST be equal-length; violations panic in debug
    /// and produce a structurally broken tree in release, so callers
    /// deserializing untrusted bytes validate first. `materials: None` means
    /// every voxel is global-0 (the storage-free `MaterialStore::Uniform`
    /// form — a serializer that dropped an all-zero material section).
    #[must_use]
    #[allow(clippy::vec_box)] // the splice-friendly box-per-leaf shape, as stored
    pub fn from_parts(
        resolution: Resolution,
        codes: Vec<u64>,
        leaves: Vec<LeafBrick>,
        materials: Option<Vec<Box<[u16; LEAF_VOXELS]>>>,
    ) -> Self {
        debug_assert_eq!(codes.len(), leaves.len(), "parts must be parallel");
        if let Some(materials) = &materials {
            debug_assert_eq!(codes.len(), materials.len(), "parts must be parallel");
        }
        debug_assert!(
            codes.windows(2).all(|w| w[0] < w[1]),
            "codes must be strictly ascending (canonical slot order)"
        );
        let nodes = build_levels(resolution.internal_levels(), &codes);
        Self {
            resolution,
            nodes,
            leaves,
            codes,
            materials: materials.map_or(MaterialStore::Uniform, MaterialStore::Dense),
            colors: ColorStore::None,
            topo_gen: 0,
        }
    }

    /// Assembles the sparse hierarchy **and its per-voxel materials** from a stream
    /// of `(coord, global_id)` pairs — one per occupied voxel, in any order — where
    /// `global_id` is a renderer global material id (`0` = the magenta MISSING
    /// sentinel). Voxels are binned into fixed `8³` leaves by `coord >> 3`; a
    /// repeated coord keeps the **last** `global_id` written.
    ///
    /// This is the GPU-free core of the sparse mesh-material path
    /// (`docs/materials/09-sparse-material-bridge.md`): the voxelizer's GPU compact
    /// pass yields per-occupied-voxel `(coord, global_id)`, and this turns them into
    /// a renderer-ready tree without the `O(n³)` scan of [`build`](Self::build) or a
    /// dense `n³` owner grid — so it scales to `2048³`. Accumulation is **per
    /// brick**, so host memory tracks the brick count, not the voxel count.
    ///
    /// Coords MUST be in `[0, resolution)`; the caller filters out-of-range voxels
    /// (an out-of-grid coord would otherwise plant a phantom brick). Out-of-range
    /// coords panic in debug and are skipped in release.
    #[must_use]
    pub fn from_voxels(
        resolution: Resolution,
        voxels: impl IntoIterator<Item = (VoxelCoord, u16)>,
    ) -> Self {
        // Accumulate per BRICK (occupancy bits + the 512-voxel material grid) so
        // host memory scales with bricks, not the ~tens of millions of voxels.
        let mut by_brick: std::collections::HashMap<u64, (LeafBrick, Box<[u16; LEAF_VOXELS]>)> =
            std::collections::HashMap::new();
        let mut any_nonzero = false;
        for (c, gid) in voxels {
            if !c.in_bounds(resolution) {
                debug_assert!(false, "from_voxels: {c:?} out of [0,n); caller must filter");
                continue;
            }
            any_nonzero |= gid != 0;
            let code = crate::morton::encode(c.x >> 3, c.y >> 3, c.z >> 3);
            let (leaf, mats) = by_brick
                .entry(code)
                .or_insert_with(|| (LeafBrick::EMPTY, Box::new([0u16; LEAF_VOXELS])));
            let (lx, ly, lz) = (c.x & 7, c.y & 7, c.z & 7);
            leaf.set_local(lx, ly, lz);
            mats[crate::morton::encode_brick(lx, ly, lz) as usize] = gid;
        }

        // Sort by Morton code into the parallel leaves / codes / materials arrays
        // (unique codes by construction — one entry per distinct brick).
        let mut entries: Vec<(u64, LeafBrick, Box<[u16; LEAF_VOXELS]>)> =
            by_brick.into_iter().map(|(c, (l, m))| (c, l, m)).collect();
        entries.sort_unstable_by_key(|(c, _, _)| *c);

        let codes: Vec<u64> = entries.iter().map(|(c, _, _)| *c).collect();
        let leaves: Vec<LeafBrick> = entries.iter().map(|(_, l, _)| *l).collect();
        // A stream that never coloured a voxel collapses to the storage-free
        // uniform form (the accumulation grids above were transient).
        let materials = if any_nonzero {
            MaterialStore::Dense(entries.into_iter().map(|(_, _, m)| m).collect())
        } else {
            MaterialStore::Uniform
        };
        let nodes = build_levels(resolution.internal_levels(), &codes);

        Self {
            resolution,
            nodes,
            leaves,
            codes,
            materials,
            colors: ColorStore::None,
            topo_gen: 0,
        }
    }

    /// The topology generation — bumped each time a brick appears or disappears.
    /// A [`SchoolBBuffer`](crate::SchoolBBuffer) uses it to detect a stale
    /// in-place patch (see [`patch_leaf`](crate::SchoolBBuffer::patch_leaf)).
    #[must_use]
    pub fn topology_generation(&self) -> u64 {
        self.topo_gen
    }

    /// Sets or clears voxel `c`, updating the structure incrementally, and
    /// reports what changed (see [`Edit`]). An out-of-bounds or no-op edit
    /// returns [`Edit::Unchanged`].
    ///
    /// In-place edits (a brick that stays non-empty) are `O(1)`; topology edits
    /// (a brick appearing/disappearing) splice the sorted leaf/code arrays and
    /// rebuild the node levels in `O(bricks)` — skipping the `O(n³)` occupancy
    /// scan that dominates a full [`build`](Self::build).
    pub fn set_voxel(&mut self, c: VoxelCoord, occupied: bool) -> Edit {
        self.edit_occupancy(c, occupied, DEFAULT_COLOR, false)
    }

    /// The occupancy-edit core shared by [`set_voxel`](Self::set_voxel) and
    /// [`set_voxel_colored`](Self::set_voxel_colored). On a truecolor tree it
    /// keeps the touched leaf's colour page length-consistent with occupancy: a
    /// newly-set voxel takes `new_color`, a cleared voxel's colour is spliced out.
    /// When `recolor` is true, an already-occupied voxel is recoloured to
    /// `new_color` ([`Edit::Color`]). On a non-truecolor tree the colour arguments
    /// are inert and this is exactly the original occupancy edit.
    fn edit_occupancy(
        &mut self,
        c: VoxelCoord,
        occupied: bool,
        new_color: u32,
        recolor: bool,
    ) -> Edit {
        if !c.in_bounds(self.resolution) {
            return Edit::Unchanged;
        }
        let code = crate::morton::encode(c.x >> 3, c.y >> 3, c.z >> 3);
        let (lx, ly, lz) = (c.x & 7, c.y & 7, c.z & 7);
        let m = crate::morton::encode_brick(lx, ly, lz);

        match self.codes.binary_search(&code) {
            Ok(idx) => {
                if self.leaves[idx].get_local(lx, ly, lz) == occupied {
                    // No occupancy change. `set_voxel_colored` on an already-
                    // occupied voxel recolours it in place; a bare set is a no-op.
                    if occupied && recolor && self.has_colors() {
                        let rank = self.leaves[idx].occupied_rank(m) as usize;
                        if let ColorStore::PerVoxel(cs) = &mut self.colors {
                            if cs.recolor(idx, rank, new_color) {
                                return Edit::Color {
                                    leaf: leaf_index(idx),
                                };
                            }
                        }
                    }
                    return Edit::Unchanged;
                }
                if occupied {
                    self.leaves[idx].set_local(lx, ly, lz);
                    // STALE-BITS FIX: a newly-set voxel must read the leaf's
                    // default material (0), not whatever the previous tenant of
                    // this Morton slot left behind — keeps "every occupied voxel
                    // has a defined material" true after every in-place SET.
                    // (`Uniform` is already all-0 by definition.)
                    if let MaterialStore::Dense(grids) = &mut self.materials {
                        grids[idx][m as usize] = 0;
                    }
                    // Splice the voxel's colour in at its rank (occupied_rank(m)
                    // is unaffected by bit m, so computing it post-set is correct).
                    if self.has_colors() {
                        let rank = self.leaves[idx].occupied_rank(m) as usize;
                        if let ColorStore::PerVoxel(cs) = &mut self.colors {
                            cs.insert_voxel(idx, rank, new_color);
                        }
                    }
                    Edit::Leaf(leaf_index(idx))
                } else {
                    // Capture the cleared voxel's colour rank before splicing the
                    // occupancy (bit m does not affect occupied_rank(m)).
                    let rank = if self.has_colors() {
                        Some(self.leaves[idx].occupied_rank(m) as usize)
                    } else {
                        None
                    };
                    self.leaves[idx].clear_local(lx, ly, lz);
                    if self.leaves[idx].is_empty() {
                        // Last voxel removed → the brick disappears (topology).
                        self.leaves.remove(idx);
                        self.codes.remove(idx);
                        self.materials.remove(idx); // splice in lockstep
                        if let ColorStore::PerVoxel(cs) = &mut self.colors {
                            cs.remove_leaf(idx);
                        }
                        self.rebuild_levels();
                        Edit::Topology
                    } else {
                        if let (Some(rank), ColorStore::PerVoxel(cs)) = (rank, &mut self.colors) {
                            cs.remove_voxel(idx, rank);
                        }
                        Edit::Leaf(leaf_index(idx))
                    }
                }
            }
            Err(insert_at) => {
                if !occupied {
                    return Edit::Unchanged; // clearing a voxel in an empty brick
                }
                // A new brick appears (topology).
                let mut leaf = LeafBrick::EMPTY;
                leaf.set_local(lx, ly, lz);
                self.leaves.insert(insert_at, leaf);
                self.codes.insert(insert_at, code);
                // A fresh all-default material grid, spliced at the same index so
                // `materials` stays parallel to `leaves`/`codes`.
                self.materials.insert_default(insert_at);
                // The colour store (if any) splices a fresh single-voxel page in
                // lockstep so it stays index-parallel too.
                if let ColorStore::PerVoxel(cs) = &mut self.colors {
                    cs.insert_leaf(insert_at, new_color);
                }
                self.rebuild_levels();
                Edit::Topology
            }
        }
    }

    /// Assigns `global_id` (an index into the global material/colour table) to
    /// the voxel at `coord`. The voxel **must be occupied**; colouring an empty
    /// or out-of-bounds voxel is a no-op ([`Edit::Unchanged`]), as is recolouring
    /// to the same id. Returns [`Edit::Material`]; `spilled` is set when the leaf
    /// now exceeds `P_CAP` distinct occupied materials — a topology-class event
    /// that bumps the generation so the adapter re-uploads the whole structure.
    ///
    /// The occupancy bitmask is never touched, so a material edit cannot change
    /// the structure's topology or regress traversal.
    pub fn set_material(&mut self, coord: VoxelCoord, global_id: u16) -> Edit {
        if !coord.in_bounds(self.resolution) {
            return Edit::Unchanged;
        }
        let code = crate::morton::encode(coord.x >> 3, coord.y >> 3, coord.z >> 3);
        let (lx, ly, lz) = (coord.x & 7, coord.y & 7, coord.z & 7);
        let Ok(idx) = self.codes.binary_search(&code) else {
            return Edit::Unchanged; // no brick here
        };
        if !self.leaves[idx].get_local(lx, ly, lz) {
            return Edit::Unchanged; // colouring an empty voxel
        }
        let m = crate::morton::encode_brick(lx, ly, lz) as usize;
        if self.materials.of(idx)[m] == global_id {
            return Edit::Unchanged; // already this colour
        }
        let was_spilled = leaf_spills(&self.leaves[idx], self.materials.of(idx));
        // Reaching here with a `Uniform` store means `global_id != 0`: the
        // first real colour materializes the dense grids (one-way).
        self.materials.densify(self.leaves.len())[idx][m] = global_id;
        let now_spilled = leaf_spills(&self.leaves[idx], self.materials.of(idx));
        if was_spilled != now_spilled {
            // Crossing the spill boundary changes the GPU layout for this leaf —
            // a topology-class event; bump the generation so the adapter does a
            // full reupload rather than an O(1) slot patch.
            self.topo_gen = self.topo_gen.wrapping_add(1);
        }
        Edit::Material {
            leaf: leaf_index(idx),
            spilled: now_spilled,
        }
    }

    /// Sets or clears voxel `c` like [`set_voxel`](Self::set_voxel), but on a
    /// truecolor tree a newly-set voxel takes `rgba` (sRGB RGBA8, R low) and an
    /// already-occupied voxel is recoloured to it (returning [`Edit::Color`]).
    /// Clearing ignores `rgba`. On a non-truecolor tree the colour is inert and
    /// the result is exactly [`set_voxel`](Self::set_voxel)'s.
    pub fn set_voxel_colored(&mut self, c: VoxelCoord, occupied: bool, rgba: u32) -> Edit {
        self.edit_occupancy(c, occupied, rgba, true)
    }

    /// Recolours the occupied voxel at `coord` to `rgba` (sRGB RGBA8, R low). The
    /// voxel **must be occupied** and the scene truecolor
    /// ([`has_colors`](Self::has_colors) true); otherwise this is a no-op
    /// ([`Edit::Unchanged`]), as is recolouring to the same value. Occupancy is
    /// never touched, so this cannot change topology.
    pub fn set_color(&mut self, coord: VoxelCoord, rgba: u32) -> Edit {
        if !self.has_colors() || !coord.in_bounds(self.resolution) {
            return Edit::Unchanged;
        }
        let code = crate::morton::encode(coord.x >> 3, coord.y >> 3, coord.z >> 3);
        let (lx, ly, lz) = (coord.x & 7, coord.y & 7, coord.z & 7);
        let Ok(idx) = self.codes.binary_search(&code) else {
            return Edit::Unchanged; // no brick here
        };
        if !self.leaves[idx].get_local(lx, ly, lz) {
            return Edit::Unchanged; // colouring an empty voxel
        }
        let rank = self.leaves[idx].occupied_rank(crate::morton::encode_brick(lx, ly, lz)) as usize;
        if let ColorStore::PerVoxel(cs) = &mut self.colors {
            if cs.recolor(idx, rank, rgba) {
                return Edit::Color {
                    leaf: leaf_index(idx),
                };
            }
        }
        Edit::Unchanged
    }

    /// Snapshots the brick with Morton code `code` — occupancy, materials,
    /// colours — or `None` if no such brick exists. The capture side of the
    /// undo system's pre/post images ([`replace_bricks`](Self::replace_bricks)
    /// is the restore side).
    #[must_use]
    pub fn brick_image(&self, code: u64) -> Option<BrickImage> {
        let idx = self.codes.binary_search(&code).ok()?;
        Some(BrickImage {
            occ: self.leaves[idx],
            materials: match &self.materials {
                MaterialStore::Uniform => None,
                MaterialStore::Dense(grids) => Some(grids[idx].clone()),
            },
            colors: match &self.colors {
                ColorStore::None => None,
                ColorStore::PerVoxel(cs) => Some(cs.leaves[idx].data.clone()),
            },
        })
    }

    /// Restores a batch of bricks to captured images: for each `(code, image)`,
    /// `Some` inserts or overwrites the brick wholesale (occupancy, materials,
    /// colours — with a fresh pool page where the size class changed) and `None`
    /// removes it if present. All splices land first; the node levels rebuild
    /// **once** at the end (and the topology generation bumps once) iff any
    /// brick appeared or disappeared — so restoring a many-brick stroke costs
    /// one rebuild, like a single topology stamp.
    ///
    /// On a truecolor tree an image captured without colours restores as the
    /// default colour (`DEFAULT_COLOR`) per occupied voxel — colour arrays stay
    /// length-consistent with occupancy by construction; on a colourless tree
    /// any image colours are inert. An image with empty occupancy is treated as
    /// a removal — an empty brick is never stored.
    pub fn replace_bricks(&mut self, bricks: impl IntoIterator<Item = (u64, Option<BrickImage>)>) {
        let mut topology = false;
        for (code, image) in bricks {
            match image {
                None => topology |= self.remove_brick(code),
                Some(img) if img.occ.is_empty() => topology |= self.remove_brick(code),
                Some(img) => {
                    let occ_count = img.occ.count_occupied() as usize;
                    let colors = |imgc: Option<Vec<u32>>| {
                        imgc.unwrap_or_else(|| vec![DEFAULT_COLOR; occ_count])
                    };
                    match self.codes.binary_search(&code) {
                        Ok(idx) => {
                            self.leaves[idx] = img.occ;
                            match img.materials {
                                Some(grid) => self.materials.densify(self.leaves.len())[idx] = grid,
                                None => {
                                    if let MaterialStore::Dense(grids) = &mut self.materials {
                                        *grids[idx] = [0u16; LEAF_VOXELS];
                                    }
                                }
                            }
                            if let ColorStore::PerVoxel(cs) = &mut self.colors {
                                let data = colors(img.colors);
                                debug_assert_eq!(data.len(), occ_count, "colours track occupancy");
                                cs.replace_leaf_data(idx, data);
                            }
                        }
                        Err(at) => {
                            self.leaves.insert(at, img.occ);
                            self.codes.insert(at, code);
                            match img.materials {
                                // Densify against the pre-insert count (the new
                                // slot's grid arrives via the splice below).
                                Some(grid) => {
                                    self.materials
                                        .densify(self.leaves.len() - 1)
                                        .insert(at, grid);
                                }
                                None => self.materials.insert_default(at),
                            }
                            if let ColorStore::PerVoxel(cs) = &mut self.colors {
                                let data = colors(img.colors);
                                debug_assert_eq!(data.len(), occ_count, "colours track occupancy");
                                cs.insert_leaf_data(at, data);
                            }
                            topology = true;
                        }
                    }
                }
            }
        }
        if topology {
            self.rebuild_levels();
        }
    }

    /// Removes the brick with Morton code `code` if present (splicing all
    /// parallel stores), returning whether anything was removed. Levels are
    /// **not** rebuilt — the caller batches that.
    fn remove_brick(&mut self, code: u64) -> bool {
        let Ok(idx) = self.codes.binary_search(&code) else {
            return false;
        };
        self.leaves.remove(idx);
        self.codes.remove(idx);
        self.materials.remove(idx);
        if let ColorStore::PerVoxel(cs) = &mut self.colors {
            cs.remove_leaf(idx);
        }
        true
    }

    /// Whether this scene carries an editable per-voxel truecolor store (set by
    /// [`install_colors`](Self::install_colors)). Fixtures, palette scenes, and
    /// occupancy-only scenes are `false`.
    #[must_use]
    pub fn has_colors(&self) -> bool {
        matches!(self.colors, ColorStore::PerVoxel(_))
    }

    /// The colour (sRGB RGBA8, R low) at `coord`, or `None` if the voxel is
    /// empty, out of bounds, or the scene is not truecolor.
    #[must_use]
    pub fn color_at(&self, coord: VoxelCoord) -> Option<u32> {
        let ColorStore::PerVoxel(cs) = &self.colors else {
            return None;
        };
        if !coord.in_bounds(self.resolution) {
            return None;
        }
        let code = crate::morton::encode(coord.x >> 3, coord.y >> 3, coord.z >> 3);
        let idx = self.codes.binary_search(&code).ok()?;
        let (lx, ly, lz) = (coord.x & 7, coord.y & 7, coord.z & 7);
        if !self.leaves[idx].get_local(lx, ly, lz) {
            return None;
        }
        let rank = self.leaves[idx].occupied_rank(crate::morton::encode_brick(lx, ly, lz)) as usize;
        Some(cs.leaves[idx].data[rank])
    }

    /// Leaf `idx`'s rank-order colours (sRGB RGBA8, R low) — the source the GPU
    /// colour-page upload derives from — or `None` if the scene is not truecolor.
    #[must_use]
    pub fn leaf_colors(&self, idx: usize) -> Option<&[u32]> {
        match &self.colors {
            ColorStore::None => None,
            ColorStore::PerVoxel(cs) => Some(&cs.leaves[idx].data),
        }
    }

    /// Installs an editable truecolor store from a stream of colours (sRGB RGBA8,
    /// R low) in the canonical **slot × intra-brick-Morton (rank)** order —
    /// exactly the order [`assemble_leaf_color`] and the scene blob emit, so the
    /// truecolor deserialize seam feeds this directly. Each leaf is assigned a
    /// pool page canonically (dense from zero), so with zero class waste the page
    /// offsets equal the old prefix-sum `leaf_color_base`. Replaces any existing
    /// colour store.
    ///
    /// # Panics
    /// Panics unless the stream holds exactly
    /// [`occupied_voxels`](Self::occupied_voxels) colours.
    ///
    /// [`assemble_leaf_color`]: crate::SchoolBBuffer::assemble_leaf_color
    pub fn install_colors(&mut self, colors: impl ExactSizeIterator<Item = u32>) {
        self.install_colors_with_chunk(colors, color_pool::CHUNK_ENTRIES);
    }

    /// [`install_colors`](Self::install_colors) with an explicit pool chunk size —
    /// for tests that force a tiny chunk to drive the GPU's `N > 1` cross-chunk
    /// path on a small scene (the pool chunk must match the renderer's
    /// `PER_CHUNK`, which is read from [`ColorPages::chunk_entries`]). Production
    /// calls [`install_colors`](Self::install_colors).
    #[doc(hidden)]
    pub fn install_colors_with_chunk(
        &mut self,
        colors: impl ExactSizeIterator<Item = u32>,
        chunk_entries: u64,
    ) {
        assert_eq!(
            colors.len() as u64,
            self.occupied_voxels(),
            "install_colors: one colour per occupied voxel"
        );
        let mut pool = ColorPool::with_chunk_entries(chunk_entries);
        let mut cleaves: Vec<ColorLeaf> = Vec::with_capacity(self.leaves.len());
        let mut transparent_leaves = 0u32;
        let mut it = colors;
        for leaf in &self.leaves {
            let occ = leaf.count_occupied();
            let data: Vec<u32> = it.by_ref().take(occ as usize).collect();
            debug_assert_eq!(
                data.len(),
                occ as usize,
                "colour stream shorter than occupancy"
            );
            let class = color_pool::class_for(occ);
            let page = pool.alloc(class);
            let transparent = data.iter().copied().any(is_transparent);
            if transparent {
                transparent_leaves += 1;
            }
            cleaves.push(ColorLeaf {
                data,
                page,
                class,
                transparent,
            });
        }
        self.colors = ColorStore::PerVoxel(Box::new(ColorLeaves {
            leaves: cleaves,
            pool,
            transparent_leaves,
        }));
    }

    /// Promotes a palette/fixture scene to the editable truecolor store
    /// (brush-editing Stage D, `docs/design/brush-editing/06`): bakes
    /// `color_of(coord, material)` for every occupied voxel — walked in the
    /// canonical slot × intra-brick-Morton (rank) order — into a freshly
    /// installed colour store, then **drops the dense material store**
    /// (colours supersede it, returning ~1 KiB/leaf on densified scenes).
    /// One-way; a no-op on a tree that already carries colours.
    pub fn promote_colors(&mut self, mut color_of: impl FnMut(VoxelCoord, u16) -> u32) {
        if self.has_colors() {
            return;
        }
        // Intra-brick Morton → local-offset LUT (the inverse of encode_brick),
        // so the walk yields rank order directly.
        let mut lut = [(0u32, 0u32, 0u32); LEAF_VOXELS];
        for lz in 0..8u32 {
            for ly in 0..8u32 {
                for lx in 0..8u32 {
                    lut[crate::morton::encode_brick(lx, ly, lz) as usize] = (lx, ly, lz);
                }
            }
        }
        let mut colors = Vec::with_capacity(usize::try_from(self.occupied_voxels()).unwrap_or(0));
        for idx in 0..self.leaves.len() {
            let origin = self.leaf_origin(idx);
            let grid = self.materials.of(idx);
            let leaf = self.leaves[idx];
            for (m, &(lx, ly, lz)) in lut.iter().enumerate() {
                if leaf.get_local(lx, ly, lz) {
                    let coord = VoxelCoord::new(origin.x + lx, origin.y + ly, origin.z + lz);
                    colors.push(color_of(coord, grid[m]));
                }
            }
        }
        self.install_colors(colors.into_iter());
        self.materials = MaterialStore::Uniform;
    }

    /// A read view of the editable colour pool — per-leaf page offset, class, and
    /// rank-order colours, plus the pool extent — or `None` if not truecolor. For
    /// the GPU upload and the School-B page-table derivation.
    #[must_use]
    pub fn color_pages(&self) -> Option<ColorPages<'_>> {
        match &self.colors {
            ColorStore::None => None,
            ColorStore::PerVoxel(cs) => Some(ColorPages { store: cs }),
        }
    }

    /// The per-voxel material grid of leaf `idx` (intra-brick Morton order) — the
    /// source the GPU upload derives the packed per-leaf palette from. `0` is the
    /// default / MISSING sentinel. A never-coloured tree reads the shared
    /// all-zero grid (see `MaterialStore`).
    #[must_use]
    pub fn leaf_materials(&self, idx: usize) -> &[u16; LEAF_VOXELS] {
        debug_assert!(idx < self.leaves.len(), "leaf index {idx} out of range");
        self.materials.of(idx)
    }

    /// Whether the material store is still the storage-free uniform form
    /// (every occupied voxel global-0) — the invariant fixture/noise scenes
    /// rely on for their memory footprint.
    #[must_use]
    pub fn has_uniform_materials(&self) -> bool {
        matches!(self.materials, MaterialStore::Uniform)
    }

    /// The material global id at `coord` (`0` if empty / out of bounds).
    #[must_use]
    pub fn material_at(&self, coord: VoxelCoord) -> u16 {
        if !coord.in_bounds(self.resolution) {
            return 0;
        }
        let code = crate::morton::encode(coord.x >> 3, coord.y >> 3, coord.z >> 3);
        match self.codes.binary_search(&code) {
            Ok(idx) => {
                let m = crate::morton::encode_brick(coord.x & 7, coord.y & 7, coord.z & 7) as usize;
                self.materials.of(idx)[m]
            }
            Err(_) => 0,
        }
    }

    /// The leaf slot (index into `leaves`, and into the School-B `leaf_mat` /
    /// `leaf_bounds` buffers) whose brick contains `coord`, or `None` if no brick
    /// is stored there. Maps a world voxel to its material slot.
    #[must_use]
    pub fn leaf_slot_of(&self, coord: VoxelCoord) -> Option<u32> {
        if !coord.in_bounds(self.resolution) {
            return None;
        }
        let code = crate::morton::encode(coord.x >> 3, coord.y >> 3, coord.z >> 3);
        self.codes.binary_search(&code).ok().map(leaf_index)
    }

    /// The voxel-space origin (min corner) of leaf `idx`'s 8³ brick —
    /// `decode(code) · 8`. Lets a post-build assembler map a leaf's local
    /// `(x, y, z)` back to a world voxel (the inverse of [`leaf_slot_of`]).
    ///
    /// [`leaf_slot_of`]: Self::leaf_slot_of
    #[must_use]
    pub fn leaf_origin(&self, idx: usize) -> VoxelCoord {
        let brick = crate::morton::decode(self.codes[idx]);
        VoxelCoord::new(brick.x * 8, brick.y * 8, brick.z * 8)
    }

    /// Bulk-assigns a material to every **occupied** voxel via
    /// `f(world_coord) -> global_id`, writing the per-leaf side-array directly (no
    /// per-voxel binary search). Unoccupied voxels keep the default global-0. This
    /// is the one-pass colouring path a builder uses after construction — the
    /// voxelizer's `owner_id → material_id` resolution feeds it. Occupancy is
    /// untouched, so it cannot change topology (no generation bump).
    pub fn fill_materials(&mut self, f: impl Fn(VoxelCoord) -> u16) {
        // Bulk colouring is what dense storage exists for; materialize it.
        let grids = self.materials.densify(self.codes.len());
        for (idx, mat) in grids.iter_mut().enumerate() {
            let origin = crate::morton::decode(self.codes[idx]); // brick coords
            let (ox, oy, oz) = (origin.x * 8, origin.y * 8, origin.z * 8);
            let leaf = self.leaves[idx]; // Copy — ends the immutable borrow of leaves
            for z in 0..8u32 {
                for y in 0..8u32 {
                    for x in 0..8u32 {
                        if leaf.get_local(x, y, z) {
                            let m = crate::morton::encode_brick(x, y, z) as usize;
                            mat[m] = f(VoxelCoord::new(ox + x, oy + y, oz + z));
                        }
                    }
                }
            }
        }
    }

    /// Rebuilds the internal node levels from the current `codes` (scan-free)
    /// and bumps the topology generation (leaf indices have changed).
    fn rebuild_levels(&mut self) {
        self.nodes = build_levels(self.resolution.internal_levels(), &self.codes);
        self.topo_gen = self.topo_gen.wrapping_add(1);
    }

    /// The grid resolution.
    #[must_use]
    pub fn resolution(&self) -> Resolution {
        self.resolution
    }

    /// The coarsest level index (`COARSE = k + 1`).
    #[must_use]
    pub fn coarse_level(&self) -> u32 {
        self.resolution.internal_levels() + 1
    }

    /// Total stored nodes across all internal levels.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.iter().map(Vec::len).sum()
    }

    /// Number of stored (non-empty) leaf bricks.
    #[must_use]
    pub fn leaf_count(&self) -> usize {
        self.leaves.len()
    }

    /// Stored `4³` nodes at internal level `L` (`0` for the voxel/leaf levels).
    #[must_use]
    pub fn nodes_at_level(&self, level: u32) -> usize {
        self.nodes.get(level as usize).map_or(0, Vec::len)
    }

    /// Total occupied voxels — the finest-level count `N(0)`.
    #[must_use]
    pub fn occupied_voxels(&self) -> u64 {
        self.leaves
            .iter()
            .map(|l| u64::from(l.count_occupied()))
            .sum()
    }

    /// The tight world-space bounding box of the occupied voxels as inclusive
    /// `(min, max)` corners, or `None` when the tree is empty.
    ///
    /// One linear pass over the leaves — each contributes its brick origin plus
    /// its own occupied-voxel bounds — so this is `O(leaf_count)` with no
    /// per-voxel work: cheap enough to compute once when a scene is installed
    /// (e.g. to pivot the turntable on the object rather than the whole grid).
    /// A built structure never stores an empty leaf, so each leaf's bounds are
    /// tight (not the conservative full brick).
    #[must_use]
    pub fn occupied_bbox(&self) -> Option<(VoxelCoord, VoxelCoord)> {
        let mut min = [u32::MAX; 3];
        let mut max = [0u32; 3];
        for (code, leaf) in self.codes.iter().zip(&self.leaves) {
            let brick = crate::morton::decode(*code);
            let origin = [brick.x * 8, brick.y * 8, brick.z * 8];
            let bounds = leaf.occupied_bounds();
            for axis in 0..3 {
                min[axis] = min[axis].min(origin[axis] + bounds.min[axis]);
                max[axis] = max[axis].max(origin[axis] + bounds.max[axis]);
            }
        }
        (!self.leaves.is_empty()).then(|| {
            (
                VoxelCoord::new(min[0], min[1], min[2]),
                VoxelCoord::new(max[0], max[1], max[2]),
            )
        })
    }

    /// Internal nodes at level `L` (empty for the voxel/leaf levels). Used by
    /// the School-B re-serialization.
    pub(crate) fn level_nodes(&self, level: u32) -> &[GpuNode] {
        match self.nodes.get(level as usize) {
            Some(v) => v,
            None => &[],
        }
    }

    /// The Morton-sorted leaf array, shared by both layouts.
    pub(crate) fn leaves_slice(&self) -> &[LeafBrick] {
        &self.leaves
    }

    /// Point query: whether voxel `c` is occupied, by descending the tree and
    /// testing the leaf bit. The independent check on the build + `popcount`
    /// addressing.
    #[must_use]
    pub fn is_occupied(&self, c: VoxelCoord) -> bool {
        if self.leaves.is_empty() || !c.in_bounds(self.resolution) {
            return false;
        }
        let (bx, by, bz) = (c.x >> 3, c.y >> 3, c.z >> 3);
        let mut level = self.coarse_level();
        let mut idx = 0usize;
        while level >= 2 {
            let node = self.nodes[level as usize][idx];
            let shift = 2 * (level - 2);
            let bit = node::child_bit((bx >> shift) & 3, (by >> shift) & 3, (bz >> shift) & 3);
            if !node.has_child(bit) {
                return false;
            }
            idx = node.child_slot(bit) as usize;
            level -= 1;
        }
        self.leaves[idx].get_local(c.x & 7, c.y & 7, c.z & 7)
    }

    /// Ray traversal (N-level hierarchical Amanatides–Woo). Returns the first
    /// occupied voxel, identical to the Tier-A oracle on the same field.
    ///
    /// Delegates to the layout-agnostic [`crate::layout::traverse`] over this
    /// School-A layout; the School-B buffer runs the exact same traversal.
    #[must_use]
    pub fn traverse(&self, ray: &Ray) -> Option<Hit> {
        crate::layout::traverse(self, ray)
    }

    /// Like [`traverse`](Self::traverse) but also returns the per-ray
    /// [`TraversalStats`] used by the §10 descent-frequency measurement.
    #[must_use]
    pub fn traverse_counted(&self, ray: &Ray) -> (Option<Hit>, TraversalStats) {
        crate::layout::traverse_counted(self, ray)
    }
}

impl NodeLayout for SparseTree {
    fn resolution(&self) -> Resolution {
        self.resolution
    }

    fn root(&self) -> Cell {
        if self.leaves.is_empty() {
            Cell::Empty
        } else if self.resolution.internal_levels() == 0 {
            Cell::Leaf(0)
        } else {
            Cell::Node(0)
        }
    }

    fn child(&self, node: u32, level: u32, child_bit: u32) -> Cell {
        let n = self.nodes[level as usize][node as usize];
        if !n.has_child(child_bit) {
            return Cell::Empty;
        }
        let slot = n.child_slot(child_bit);
        if level == 2 {
            Cell::Leaf(slot)
        } else {
            Cell::Node(slot)
        }
    }

    fn leaf(&self, idx: u32) -> &LeafBrick {
        &self.leaves[idx as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BitGrid;
    use crate::fixtures::{Checkerboard, Empty, OctantFractal, SingleVoxel, Solid};
    use crate::oracle;
    use glam::DVec3;

    fn res(n: u32) -> Resolution {
        Resolution::new(n).unwrap()
    }

    /// The dense grid count, asserting the store has actually densified.
    fn dense_len(tree: &SparseTree) -> usize {
        match &tree.materials {
            MaterialStore::Uniform => panic!("expected dense materials"),
            MaterialStore::Dense(grids) => grids.len(),
        }
    }

    #[test]
    fn occupied_bbox_is_the_tight_world_box_or_none_when_empty() {
        // Empty tree → no box.
        let empty = SparseTree::build(&Empty {
            resolution: res(32),
        });
        assert_eq!(empty.occupied_bbox(), None);

        // Two voxels in different bricks: the box spans both, tightly (not
        // brick-granular — bounds come from the per-leaf occupied extent).
        let tree = SparseTree::from_voxels(
            res(128),
            [
                (VoxelCoord::new(3, 5, 7), 0u16),
                (VoxelCoord::new(40, 12, 9), 0),
            ],
        );
        assert_eq!(
            tree.occupied_bbox(),
            Some((VoxelCoord::new(3, 5, 7), VoxelCoord::new(40, 12, 9))),
        );

        // A single voxel is a degenerate (min == max) box, not an error.
        let one = SparseTree::from_voxels(res(32), [(VoxelCoord::new(9, 9, 9), 0u16)]);
        assert_eq!(
            one.occupied_bbox(),
            Some((VoxelCoord::new(9, 9, 9), VoxelCoord::new(9, 9, 9))),
        );
    }

    #[test]
    fn builds_and_zero_streams_stay_uniform() {
        // Fixture builds and all-zero voxel streams must not pay 1 KiB/leaf.
        let tree = SparseTree::build(&Solid {
            resolution: res(32),
        });
        assert!(tree.has_uniform_materials());
        assert_eq!(tree.material_at(VoxelCoord::new(1, 2, 3)), 0);
        assert_eq!(tree.leaf_materials(0), &[0u16; LEAF_VOXELS]);

        let streamed = SparseTree::from_voxels(
            res(32),
            [
                (VoxelCoord::new(1, 2, 3), 0u16),
                (VoxelCoord::new(30, 31, 9), 0),
            ],
        );
        assert!(streamed.has_uniform_materials());

        let coloured = SparseTree::from_voxels(res(32), [(VoxelCoord::new(1, 2, 3), 7u16)]);
        assert!(!coloured.has_uniform_materials());
    }

    #[test]
    fn uniform_survives_topology_edits_and_densifies_on_first_colour() {
        let mut tree = SparseTree::build(&Empty {
            resolution: res(32),
        });
        let v = VoxelCoord::new;
        // Adds, in-place sets, and removals never need dense storage.
        assert_eq!(tree.set_voxel(v(0, 0, 0), true), Edit::Topology);
        assert_eq!(tree.set_voxel(v(1, 0, 0), true), Edit::Leaf(0));
        assert_eq!(tree.set_voxel(v(8, 8, 8), true), Edit::Topology);
        assert_eq!(tree.set_voxel(v(8, 8, 8), false), Edit::Topology);
        assert!(tree.has_uniform_materials());
        // Recolouring to the default is a no-op, not a densify.
        assert_eq!(tree.set_material(v(0, 0, 0), 0), Edit::Unchanged);
        assert!(tree.has_uniform_materials());
        // The first real colour is the one-way door.
        assert!(matches!(
            tree.set_material(v(0, 0, 0), 5),
            Edit::Material { .. }
        ));
        assert!(!tree.has_uniform_materials());
        assert_eq!(dense_len(&tree), tree.leaf_count());
        assert_eq!(tree.material_at(v(0, 0, 0)), 5);
        assert_eq!(
            tree.material_at(v(1, 0, 0)),
            0,
            "other voxels keep the default"
        );
    }

    #[test]
    fn uniform_and_dense_zero_trees_are_observably_identical() {
        // Differential: a uniform tree and its dense-zero twin must agree on
        // every read the GPU upload path makes.
        let uniform = SparseTree::build(&Checkerboard {
            resolution: res(32),
        });
        assert!(uniform.has_uniform_materials());
        let zeros = (0..uniform.leaf_count())
            .map(|_| Box::new([0u16; LEAF_VOXELS]))
            .collect();
        let dense = SparseTree::from_parts(
            res(32),
            uniform.codes.clone(),
            uniform.leaves.clone(),
            Some(zeros),
        );
        assert!(!dense.has_uniform_materials());
        for idx in 0..uniform.leaf_count() {
            assert_eq!(uniform.leaf_materials(idx), dense.leaf_materials(idx));
        }
        let (ub, db) = (
            crate::SchoolBBuffer::from_sparse(&uniform),
            crate::SchoolBBuffer::from_sparse(&dense),
        );
        assert_eq!(ub.nodes(), db.nodes());
        assert_eq!(ub.leaves(), db.leaves());
        assert_eq!(ub.leaf_mat_words(), db.leaf_mat_words());
        assert_eq!(ub.leaf_bounds_words(), db.leaf_bounds_words());
    }

    fn splitmix64(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    #[allow(clippy::cast_precision_loss)]
    fn unit(state: &mut u64) -> f64 {
        (splitmix64(state) >> 11) as f64 / (1u64 << 53) as f64
    }

    #[test]
    fn from_parts_round_trips_a_decomposed_tree() {
        // Build a tree with real materials, decompose it exactly the way a
        // serializer walks it (slot order), reassemble, and compare.
        let resolution = res(32);
        let original = SparseTree::from_voxels(
            resolution,
            [
                (VoxelCoord::new(1, 2, 3), 7u16),
                (VoxelCoord::new(1, 3, 3), 7),
                (VoxelCoord::new(30, 2, 9), 42),
                (VoxelCoord::new(0, 31, 31), 1),
            ],
        );
        let materials = (0..original.leaf_count())
            .map(|i| Box::new(*original.leaf_materials(i)))
            .collect();
        let rebuilt = SparseTree::from_parts(
            resolution,
            original.codes.clone(),
            original.leaves.clone(),
            Some(materials),
        );

        assert_eq!(rebuilt.nodes, original.nodes, "internal levels must match");
        assert_eq!(rebuilt.leaf_count(), original.leaf_count());
        assert_eq!(rebuilt.occupied_voxels(), original.occupied_voxels());
        for (c, gid) in [
            (VoxelCoord::new(1, 2, 3), 7u16),
            (VoxelCoord::new(30, 2, 9), 42),
            (VoxelCoord::new(0, 31, 31), 1),
        ] {
            assert!(rebuilt.is_occupied(c));
            assert_eq!(rebuilt.material_at(c), gid);
        }
    }

    #[test]
    fn brush_voxels_is_a_clamped_sphere() {
        // radius 0 is the single centre voxel.
        let c = VoxelCoord::new(10, 10, 10);
        assert_eq!(brush_voxels(c, 0), vec![c]);

        // Every returned voxel lies within the Euclidean radius, and a sample of
        // in-range voxels is present (membership is exactly dx²+dy²+dz² ≤ r²).
        let r = 3u32;
        let got = brush_voxels(c, r);
        let r2 = i64::from(r) * i64::from(r);
        for v in &got {
            let (dx, dy, dz) = (
                i64::from(v.x) - 10,
                i64::from(v.y) - 10,
                i64::from(v.z) - 10,
            );
            assert!(dx * dx + dy * dy + dz * dz <= r2, "{v:?} outside radius");
        }
        assert!(got.contains(&VoxelCoord::new(13, 10, 10))); // on the axis, |d|=r
        assert!(!got.contains(&VoxelCoord::new(13, 13, 10))); // corner, outside

        // Voxels below the origin are dropped; the rest survive for set_voxel to
        // range-check (it treats out-of-bounds as Edit::Unchanged).
        let edge = brush_voxels(VoxelCoord::new(0, 0, 0), 2);
        assert!(edge.iter().all(|v| v.x <= 2 && v.y <= 2 && v.z <= 2));
        assert!(edge.contains(&VoxelCoord::new(0, 0, 0)));
    }

    #[test]
    fn point_query_matches_field_exhaustively() {
        // Every voxel in a 128³ grid descends to the correct occupancy.
        let field = OctantFractal::sierpinski_tetrahedron(res(128));
        let tree = SparseTree::build(&field);
        let n = field.resolution().voxels_per_axis();
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    let c = VoxelCoord::new(x, y, z);
                    assert_eq!(tree.is_occupied(c), field.is_occupied(c), "voxel {c:?}");
                }
            }
        }
    }

    #[test]
    fn sparsity_drops_node_and_leaf_counts() {
        // A D=2 fractal in 512³ stores far fewer than the 64³ dense bricks.
        let field = OctantFractal::sierpinski_tetrahedron(res(512));
        let tree = SparseTree::build(&field);
        let dense_bricks = 64usize.pow(3);
        assert!(
            tree.leaf_count() < dense_bricks / 4,
            "expected sparse leaves, got {} of {dense_bricks}",
            tree.leaf_count()
        );
        assert_eq!(tree.coarse_level(), 4); // 512³: COARSE = L4
    }

    #[test]
    fn handles_single_brick_resolution() {
        // res 8 = k=0: no internal nodes, the root is the lone leaf.
        let field = SingleVoxel {
            resolution: res(8),
            voxel: VoxelCoord::new(2, 5, 1),
        };
        let tree = SparseTree::build(&field);
        assert_eq!(tree.node_count(), 0);
        assert_eq!(tree.leaf_count(), 1);
        assert!(tree.is_occupied(VoxelCoord::new(2, 5, 1)));
        assert!(!tree.is_occupied(VoxelCoord::new(2, 5, 2)));
    }

    #[test]
    fn empty_and_solid_edges() {
        let empty = SparseTree::build(&Empty {
            resolution: res(32),
        });
        assert_eq!(empty.leaf_count(), 0);
        let ray = Ray::new(DVec3::new(-1.0, 4.0, 4.0), DVec3::X);
        assert!(empty.traverse(&ray).is_none());

        let solid = SparseTree::build(&Solid {
            resolution: res(32),
        });
        let hit = solid.traverse(&ray).unwrap();
        assert_eq!(hit.voxel, VoxelCoord::new(0, 4, 4));
    }

    #[test]
    fn traverse_matches_oracle_on_random_rays() {
        let r = res(128);
        let nf = f64::from(r.voxels_per_axis());
        let checker = Checkerboard { resolution: r };
        let frac = OctantFractal::sierpinski_tetrahedron(r);
        let checker_tree = SparseTree::build(&checker);
        let frac_tree = SparseTree::build(&frac);

        let mut state = 0x1234_5678_9ABC_DEF0u64;
        let mut compared = 0u32;
        for _ in 0..4000 {
            let origin = DVec3::new(
                unit(&mut state) * (nf + 8.0) - 4.0,
                unit(&mut state) * (nf + 8.0) - 4.0,
                unit(&mut state) * (nf + 8.0) - 4.0,
            );
            let dir = DVec3::new(
                unit(&mut state) * 2.0 - 1.0,
                unit(&mut state) * 2.0 - 1.0,
                unit(&mut state) * 2.0 - 1.0,
            );
            if dir.length() < 1e-3 {
                continue;
            }
            let ray = Ray::new(origin, dir);

            for (oracle_hit, tree) in [
                (oracle::first_hit(&checker, &ray), &checker_tree),
                (oracle::first_hit(&frac, &ray), &frac_tree),
            ] {
                let tree_hit = tree.traverse(&ray);
                assert_eq!(
                    oracle_hit.is_some(),
                    tree_hit.is_some(),
                    "hit/miss, dir={dir:?}"
                );
                if let (Some(a), Some(b)) = (oracle_hit, tree_hit) {
                    assert!(
                        (a.t_enter - b.t_enter).abs() < 1e-6,
                        "t mismatch: oracle={} tree={} dir={dir:?}",
                        a.t_enter,
                        b.t_enter
                    );
                }
                compared += 1;
            }
        }
        assert!(compared > 1000);
    }

    /// The edit correctness gate: a tree mutated by a long sequence of random
    /// voxel toggles must be **byte-for-byte identical** to a fresh build of the
    /// same edited field — same leaves, same codes, same node levels — and agree
    /// with the reference field on every voxel. This is the incremental-edit
    /// analogue of the oracle differential.
    #[test]
    fn incremental_edits_match_fresh_build() {
        let r = res(32); // k = 1: exercises a real internal node level
        let n = r.voxels_per_axis();
        let base = OctantFractal::sierpinski_tetrahedron(r);
        let mut grid = BitGrid::from_field(&base); // mutable reference field
        let mut tree = SparseTree::build(&base);

        let mut state = 0xED17_0000_0000_0001u64;
        let rc = |s: &mut u64| u32::try_from(splitmix64(s) % u64::from(n)).unwrap();
        let (mut leaf_edits, mut topo_edits) = (0u32, 0u32);
        for _ in 0..4000 {
            let c = VoxelCoord::new(rc(&mut state), rc(&mut state), rc(&mut state));
            let occ = !grid.is_occupied(c); // always a real change
            if occ {
                grid.set(c);
            } else {
                grid.clear(c);
            }
            match tree.set_voxel(c, occ) {
                Edit::Leaf(_) => leaf_edits += 1,
                Edit::Topology => topo_edits += 1,
                Edit::Unchanged => panic!("a toggle must change something at {c:?}"),
                Edit::Material { .. } => unreachable!("set_voxel never changes materials"),
                Edit::Color { .. } => unreachable!("set_voxel never changes colours"),
            }
        }

        let fresh = SparseTree::build(&grid);
        assert_eq!(tree.codes, fresh.codes, "code arrays diverged");
        assert_eq!(tree.leaves, fresh.leaves, "leaf arrays diverged");
        assert_eq!(tree.nodes, fresh.nodes, "node levels diverged");
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    let c = VoxelCoord::new(x, y, z);
                    assert_eq!(tree.is_occupied(c), grid.is_occupied(c), "voxel {c:?}");
                }
            }
        }
        assert!(
            leaf_edits > 0 && topo_edits > 0,
            "want both edit kinds exercised: leaf={leaf_edits} topo={topo_edits}"
        );
    }

    /// Pins the [`Edit`] classification an adapter relies on to decide how much
    /// to re-upload.
    #[test]
    fn edit_classification_is_correct() {
        let r = res(32);
        let mut tree = SparseTree::build(&Empty { resolution: r });
        let v = VoxelCoord::new;

        // First voxel in an empty region → a brick appears (topology).
        assert_eq!(tree.set_voxel(v(0, 0, 0), true), Edit::Topology);
        // Another voxel in the same (now-occupied) brick → in-place.
        assert!(matches!(tree.set_voxel(v(1, 0, 0), true), Edit::Leaf(_)));
        // Re-setting an already-set voxel → no change.
        assert_eq!(tree.set_voxel(v(1, 0, 0), true), Edit::Unchanged);
        // Clearing one of two voxels (brick stays non-empty) → in-place.
        assert!(matches!(tree.set_voxel(v(1, 0, 0), false), Edit::Leaf(_)));
        // Clearing the last voxel → the brick disappears (topology).
        assert_eq!(tree.set_voxel(v(0, 0, 0), false), Edit::Topology);
        // Clearing a voxel in an already-empty brick → no change.
        assert_eq!(tree.set_voxel(v(0, 0, 0), false), Edit::Unchanged);
        // Out of bounds → no change.
        assert_eq!(tree.set_voxel(v(32, 0, 0), true), Edit::Unchanged);
        assert_eq!(tree.leaf_count(), 0);
    }

    /// Edits at `k = 0` (a single `8³` brick, no internal nodes): the brick is
    /// the root, so add/remove toggles the root leaf directly.
    #[test]
    fn edits_handle_single_brick_resolution() {
        let r = res(8);
        let mut tree = SparseTree::build(&Empty { resolution: r });
        let v = VoxelCoord::new;

        assert_eq!(tree.set_voxel(v(2, 3, 4), true), Edit::Topology); // root leaf appears
        assert_eq!(tree.leaf_count(), 1);
        assert_eq!(tree.node_count(), 0);
        assert!(tree.is_occupied(v(2, 3, 4)));
        assert!(matches!(tree.set_voxel(v(5, 6, 7), true), Edit::Leaf(_)));
        assert!(matches!(tree.set_voxel(v(2, 3, 4), false), Edit::Leaf(_)));
        assert_eq!(tree.set_voxel(v(5, 6, 7), false), Edit::Topology); // root leaf removed
        assert_eq!(tree.leaf_count(), 0);
        assert!(!tree.is_occupied(v(5, 6, 7)));
        // Matches a fresh build of the now-empty field.
        let empty = SparseTree::build(&Empty { resolution: r });
        assert_eq!(tree.leaves, empty.leaves);
        assert_eq!(tree.codes, empty.codes);
    }

    /// Building up from empty and tearing back down to empty both reproduce a
    /// fresh build at each end.
    #[test]
    fn edits_from_empty_and_back_to_empty() {
        let r = res(32);
        let target = OctantFractal::sierpinski_tetrahedron(r);
        let n = r.voxels_per_axis();

        // Build the target field up voxel by voxel from empty.
        let mut tree = SparseTree::build(&Empty { resolution: r });
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    let c = VoxelCoord::new(x, y, z);
                    if target.is_occupied(c) {
                        tree.set_voxel(c, true);
                    }
                }
            }
        }
        let fresh = SparseTree::build(&target);
        assert_eq!(tree.codes, fresh.codes);
        assert_eq!(tree.leaves, fresh.leaves);
        assert_eq!(tree.nodes, fresh.nodes);

        // Tear it all back down to empty.
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    tree.set_voxel(VoxelCoord::new(x, y, z), false);
                }
            }
        }
        assert_eq!(tree.leaf_count(), 0);
        assert_eq!(tree.node_count(), 0);
    }

    // ---- milestone 3: per-voxel material edit path ----

    #[test]
    fn set_material_colours_occupied_voxels() {
        let r = res(32);
        let mut tree = SparseTree::build(&Empty { resolution: r });
        let v = VoxelCoord::new;
        // Two voxels in the same brick.
        tree.set_voxel(v(0, 0, 0), true);
        tree.set_voxel(v(1, 0, 0), true);
        assert_eq!(tree.material_at(v(0, 0, 0)), 0); // default sentinel
        assert_eq!(
            tree.set_material(v(0, 0, 0), 7),
            Edit::Material {
                leaf: 0,
                spilled: false
            }
        );
        tree.set_material(v(1, 0, 0), 9);
        assert_eq!(tree.material_at(v(0, 0, 0)), 7);
        assert_eq!(tree.material_at(v(1, 0, 0)), 9);
        // Recolouring to the same id is a no-op.
        assert_eq!(tree.set_material(v(0, 0, 0), 7), Edit::Unchanged);
    }

    #[test]
    fn set_material_on_empty_or_oob_is_noop() {
        let r = res(32);
        let mut tree = SparseTree::build(&Empty { resolution: r });
        let v = VoxelCoord::new;
        tree.set_voxel(v(0, 0, 0), true);
        // Unoccupied voxel in an existing brick → no-op.
        assert_eq!(tree.set_material(v(1, 0, 0), 5), Edit::Unchanged);
        assert_eq!(tree.material_at(v(1, 0, 0)), 0);
        // Voxel in a brick that does not exist → no-op.
        assert_eq!(tree.set_material(v(16, 0, 0), 5), Edit::Unchanged);
        // Out of bounds → no-op.
        assert_eq!(tree.set_material(v(32, 0, 0), 5), Edit::Unchanged);
    }

    #[test]
    fn in_place_set_resets_stale_material() {
        // The stale-bits fix: a re-set voxel must read the default material,
        // not the colour it carried before being cleared.
        let r = res(32);
        let mut tree = SparseTree::build(&Empty { resolution: r });
        let v = VoxelCoord::new;
        tree.set_voxel(v(0, 0, 0), true);
        tree.set_voxel(v(1, 0, 0), true); // keep the brick non-empty
        tree.set_material(v(0, 0, 0), 9);
        assert_eq!(tree.material_at(v(0, 0, 0)), 9);
        // Clear then re-set voxel 0 in place (brick stays non-empty via voxel 1).
        assert!(matches!(tree.set_voxel(v(0, 0, 0), false), Edit::Leaf(_)));
        assert!(matches!(tree.set_voxel(v(0, 0, 0), true), Edit::Leaf(_)));
        assert_eq!(
            tree.material_at(v(0, 0, 0)),
            0,
            "stale material survived a re-set"
        );
    }

    #[test]
    fn materials_stay_index_parallel_across_topology() {
        let r = res(32);
        let mut tree = SparseTree::build(&Empty { resolution: r });
        let v = VoxelCoord::new;
        // Occupy a brick at local (1,0,0) and colour it (densifies the store).
        tree.set_voxel(v(8, 0, 0), true);
        tree.set_material(v(8, 0, 0), 5);
        assert_eq!(dense_len(&tree), tree.leaves.len());
        // Insert a brick at (0,0,0) — a LOWER Morton code → shifts the first
        // brick's index 0→1. The side-array must splice in lockstep.
        assert_eq!(tree.set_voxel(v(0, 0, 0), true), Edit::Topology);
        assert_eq!(dense_len(&tree), tree.leaves.len());
        assert_eq!(tree.leaf_count(), 2);
        // The colour followed its brick across the renumber; the new brick is 0.
        assert_eq!(tree.material_at(v(8, 0, 0)), 5);
        assert_eq!(tree.material_at(v(0, 0, 0)), 0);
    }

    #[test]
    fn removed_then_readded_leaf_starts_fresh() {
        let r = res(32);
        let mut tree = SparseTree::build(&Empty { resolution: r });
        let v = VoxelCoord::new;
        tree.set_voxel(v(0, 0, 0), true);
        tree.set_material(v(0, 0, 0), 9);
        // Clear the only voxel → the brick disappears (topology).
        assert_eq!(tree.set_voxel(v(0, 0, 0), false), Edit::Topology);
        assert_eq!(dense_len(&tree), 0);
        // Re-create the brick → a fresh material grid, NOT the old palette.
        assert_eq!(tree.set_voxel(v(0, 0, 0), true), Edit::Topology);
        assert_eq!(tree.material_at(v(0, 0, 0)), 0, "old material resurrected");
    }

    #[test]
    fn leaf_over_cap_spills_and_bumps_generation() {
        let r = res(32);
        let mut tree = SparseTree::build(&Empty { resolution: r });
        let v = VoxelCoord::new;
        // 17 occupied voxels in one 8³ brick. Uncolored occupied voxels carry
        // the default material 0 (the magenta sentinel), which is itself a
        // palette entry — so distinct = {0} ∪ {colours so far}. With 17 occupied
        // and 15 colours applied, two voxels remain at 0 ⇒ {0,1..15} = 16 entries
        // (inline). Colouring the 16th distinct ⇒ {0,1..16} = 17 ⇒ spill.
        let coord = |i: u32| v(i % 8, i / 8, 0); // all within the (0,0,0) brick
        for i in 0..17u32 {
            tree.set_voxel(coord(i), true);
        }
        for i in 0..15u32 {
            assert_eq!(
                tree.set_material(coord(i), u16::try_from(i + 1).unwrap()),
                Edit::Material {
                    leaf: 0,
                    spilled: false
                }
            );
        }
        let gen_before = tree.topology_generation();
        // The 16th distinct colour (with a 0 still present) makes 17 ⇒ spill,
        // a topology-class event that bumps the generation.
        assert_eq!(
            tree.set_material(coord(15), 16),
            Edit::Material {
                leaf: 0,
                spilled: true
            }
        );
        assert_eq!(tree.topology_generation(), gen_before + 1);
    }

    #[test]
    fn from_voxels_matches_incremental_build_and_carries_materials() {
        let r = res(32);
        // Voxels across two bricks (brick (0,0,0): the first three; brick (1,0,0):
        // the last two), each with a global material id.
        let pts: Vec<(VoxelCoord, u16)> = vec![
            (VoxelCoord::new(0, 0, 0), 3),
            (VoxelCoord::new(1, 0, 0), 3),
            (VoxelCoord::new(7, 7, 7), 5),
            (VoxelCoord::new(8, 0, 0), 7),
            (VoxelCoord::new(9, 2, 3), 7),
        ];
        let tree = SparseTree::from_voxels(r, pts.iter().copied());

        // Occupancy is bit-identical to building the same voxels incrementally.
        let mut inc = SparseTree::build(&Empty { resolution: r });
        for (c, _) in &pts {
            inc.set_voxel(*c, true);
        }
        assert_eq!(
            tree.leaves, inc.leaves,
            "occupancy diverged from incremental"
        );
        assert_eq!(tree.codes, inc.codes, "brick codes diverged");
        assert_eq!(tree.leaf_count(), 2);

        // Materials are carried per voxel; unoccupied reads global-0.
        for (c, gid) in &pts {
            assert_eq!(tree.material_at(*c), *gid, "material at {c:?}");
        }
        assert_eq!(tree.material_at(VoxelCoord::new(2, 0, 0)), 0);
    }

    #[test]
    fn from_voxels_duplicate_coord_keeps_last() {
        let r = res(32);
        // A repeated coord (chunk-boundary case) keeps the last global id written.
        let pts = [
            (VoxelCoord::new(4, 4, 4), 2u16),
            (VoxelCoord::new(4, 4, 4), 9u16),
        ];
        let tree = SparseTree::from_voxels(r, pts.iter().copied());
        assert_eq!(tree.leaf_count(), 1);
        assert_eq!(tree.material_at(VoxelCoord::new(4, 4, 4)), 9);
    }

    // ---- brush-editing Stage A1: the editable paged colour store -------------

    use std::collections::BTreeMap;

    /// A unique opaque colour for every voxel (packs the coord), so any
    /// slot/rank/page transpose is observable as a wrong colour.
    fn color_of(c: VoxelCoord) -> u32 {
        let byte = |v: u32| u8::try_from(v & 0xff).unwrap();
        u32::from_le_bytes([byte(c.x), byte(c.y), byte(c.z), 255])
    }

    /// The colours of `map`'s occupied voxels in the canonical slot × rank order
    /// `install_colors` expects (leaf-slot order, then intra-brick Morton).
    fn colors_in_slot_rank_order(tree: &SparseTree, map: &BTreeMap<VoxelCoord, u32>) -> Vec<u32> {
        let mut out = Vec::new();
        for idx in 0..tree.leaf_count() {
            let origin = tree.leaf_origin(idx);
            let leaf = tree.leaves[idx];
            for m in 0..512u32 {
                let l = crate::morton::decode(u64::from(m));
                if leaf.get_local(l.x, l.y, l.z) {
                    let w = VoxelCoord::new(origin.x + l.x, origin.y + l.y, origin.z + l.z);
                    out.push(map[&w]);
                }
            }
        }
        out
    }

    /// Builds a truecolor tree whose occupied set is `map`'s keys and whose
    /// colours are `map`'s values — the from-scratch reference the incremental
    /// edits are diffed against.
    fn build_colored(r: Resolution, map: &BTreeMap<VoxelCoord, u32>) -> SparseTree {
        let mut tree = SparseTree::from_voxels(r, map.keys().map(|&c| (c, 0u16)));
        let colors = colors_in_slot_rank_order(&tree, map);
        tree.install_colors(colors.into_iter());
        tree
    }

    /// The sharp read oracle: rebuild the flat GPU pool from `color_pages()` and
    /// verify that reading `pool[page[slot] + occupied_rank(morton)]` — the exact
    /// addressing the WGSL hit-read uses — recovers `color_at` for every occupied
    /// voxel. Also checks pages are disjoint and never straddle a chunk.
    fn assert_logical_pool_read(tree: &SparseTree) {
        let pages = tree.color_pages().expect("tree must be truecolor");
        let chunk = color_pool::CHUNK_ENTRIES;
        // Flat pool image.
        let mut pool = vec![0u32; usize::try_from(pages.total_entries()).unwrap()];
        let mut spans: Vec<(u64, u64)> = Vec::new();
        for i in 0..pages.len() {
            let page = u64::from(pages.page_of(i));
            let colors = pages.colors_of(i);
            let class = u64::from(pages.class_of(i));
            // Non-straddle: the whole class-sized page lives in one chunk.
            assert_eq!(
                page / chunk,
                (page + class - 1) / chunk,
                "leaf {i} page straddles a chunk"
            );
            let s = usize::try_from(page).unwrap();
            pool[s..s + colors.len()].copy_from_slice(colors);
            spans.push((page, page + class));
        }
        // Disjoint pages.
        spans.sort_unstable();
        for w in spans.windows(2) {
            assert!(
                w[0].1 <= w[1].0,
                "colour pages overlap: {:?} then {:?}",
                w[0],
                w[1]
            );
        }
        // Read every occupied voxel back the way the GPU does.
        for idx in 0..tree.leaf_count() {
            let origin = tree.leaf_origin(idx);
            let leaf = tree.leaves[idx];
            let base = pages.page_of(idx);
            for m in 0..512u32 {
                let l = crate::morton::decode(u64::from(m));
                if leaf.get_local(l.x, l.y, l.z) {
                    let w = VoxelCoord::new(origin.x + l.x, origin.y + l.y, origin.z + l.z);
                    let g = base + leaf.occupied_rank(m);
                    assert_eq!(
                        pool[g as usize],
                        tree.color_at(w).unwrap(),
                        "pool read at leaf {idx} morton {m} disagrees with color_at",
                    );
                }
            }
        }
    }

    #[test]
    fn install_colors_round_trips_via_color_at() {
        let r = res(32);
        let occ = OctantFractal::sierpinski_tetrahedron(r);
        let grid = BitGrid::from_field(&occ);
        let n = r.voxels_per_axis();
        let mut map = BTreeMap::new();
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    let c = VoxelCoord::new(x, y, z);
                    if grid.is_occupied(c) {
                        map.insert(c, color_of(c));
                    }
                }
            }
        }
        let tree = build_colored(r, &map);
        assert!(tree.has_colors());
        // leaf_colors length is index-parallel with occupancy.
        for idx in 0..tree.leaf_count() {
            assert_eq!(
                u32::try_from(tree.leaf_colors(idx).unwrap().len()).unwrap(),
                tree.leaves[idx].count_occupied(),
            );
        }
        // Every occupied voxel reads back its unique colour; empties read None.
        for (&c, &col) in &map {
            assert_eq!(tree.color_at(c), Some(col), "color_at {c:?}");
        }
        assert_eq!(
            tree.color_at(VoxelCoord::new(0, 0, 0)).is_some(),
            map.contains_key(&VoxelCoord::new(0, 0, 0))
        );
        assert_eq!(
            tree.color_at(VoxelCoord::new(n, 0, 0)),
            None,
            "out of bounds"
        );
        assert_logical_pool_read(&tree);
    }

    /// The colour-splice byte-equality oracle: a truecolor tree driven through a
    /// long random script of occupancy-only edits, colour-carrying edits, and
    /// recolours must end **byte-for-byte** equal (codes, leaves, per-leaf
    /// colours) to a fresh from-scratch install of the final field — and its
    /// flat-pool read must match `color_at` throughout.
    #[test]
    fn colour_edits_match_a_fresh_install() {
        let r = res(32);
        let base = OctantFractal::sierpinski_tetrahedron(r);
        let grid = BitGrid::from_field(&base);
        let n = r.voxels_per_axis();
        let mut map: BTreeMap<VoxelCoord, u32> = BTreeMap::new();
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    let c = VoxelCoord::new(x, y, z);
                    if grid.is_occupied(c) {
                        map.insert(c, color_of(c));
                    }
                }
            }
        }
        let mut tree = build_colored(r, &map);

        let mut state = 0xC010_ED17_0000_0001u64;
        let rc = |s: &mut u64| u32::try_from(splitmix64(s) % u64::from(n)).unwrap();
        let (mut colored_adds, mut erases, mut recolors, mut topo) = (0u32, 0u32, 0u32, 0u32);
        for i in 0..4000u32 {
            let c = VoxelCoord::new(rc(&mut state), rc(&mut state), rc(&mut state));
            let rbyte = |s: &mut u64| u8::try_from(splitmix64(s) & 0xff).unwrap();
            let rgba =
                u32::from_le_bytes([rbyte(&mut state), rbyte(&mut state), rbyte(&mut state), 255]);
            match splitmix64(&mut state) % 4 {
                0 => {
                    if matches!(tree.set_voxel_colored(c, true, rgba), Edit::Topology) {
                        topo += 1;
                    }
                    map.insert(c, rgba);
                    colored_adds += 1;
                }
                1 => {
                    if matches!(tree.set_voxel(c, false), Edit::Topology) {
                        topo += 1;
                    }
                    map.remove(&c);
                    erases += 1;
                }
                2 => {
                    if matches!(tree.set_color(c, rgba), Edit::Color { .. }) {
                        recolors += 1;
                    }
                    if let Some(v) = map.get_mut(&c) {
                        *v = rgba;
                    }
                }
                _ => {
                    tree.set_voxel(c, true);
                    map.entry(c).or_insert(DEFAULT_COLOR);
                }
            }
            // Spot-check the invariant mid-stream so a divergence is caught near
            // the op that caused it, not only at the end.
            if i % 512 == 0 {
                assert_logical_pool_read(&tree);
            }
        }

        let fresh = build_colored(r, &map);
        assert_eq!(tree.codes, fresh.codes, "codes diverged");
        assert_eq!(tree.leaves, fresh.leaves, "leaves diverged");
        for idx in 0..tree.leaf_count() {
            assert_eq!(
                tree.leaf_colors(idx),
                fresh.leaf_colors(idx),
                "leaf {idx} colours diverged from a fresh install",
            );
            // Patched-page CONTENT (not address) equals the fresh install's page.
            let (ip, fp) = (tree.color_pages().unwrap(), fresh.color_pages().unwrap());
            assert_eq!(
                color_pool::pack_page(ip.colors_of(idx), ip.class_of(idx)),
                color_pool::pack_page(fp.colors_of(idx), fp.class_of(idx)),
                "packed page content diverged at leaf {idx}",
            );
        }
        for (&c, &col) in &map {
            assert_eq!(tree.color_at(c), Some(col), "color_at {c:?} after script");
        }
        assert_logical_pool_read(&tree);
        assert!(
            colored_adds > 0 && erases > 0 && recolors > 0 && topo > 0,
            "want every edit kind: adds={colored_adds} erases={erases} recolors={recolors} topo={topo}",
        );
    }

    #[test]
    fn erasing_a_leafs_last_voxel_frees_its_colour_page() {
        // A colour topology edit (brick disappears) must splice the colour store
        // in lockstep — leaf_colors stays index-parallel and the freed page is
        // reusable (watermark does not grow when the next add reuses the class).
        let r = res(32);
        let v = VoxelCoord::new;
        let mut map = BTreeMap::new();
        map.insert(v(0, 0, 0), color_of(v(0, 0, 0)));
        map.insert(v(9, 0, 0), color_of(v(9, 0, 0))); // a second, separate brick
        let mut tree = build_colored(r, &map);
        assert_eq!(tree.leaf_count(), 2);
        // Erase the lone voxel of brick (1,0,0) → topology; colour store shrinks.
        assert_eq!(tree.set_voxel(v(9, 0, 0), false), Edit::Topology);
        assert_eq!(tree.leaf_count(), 1);
        assert_eq!(tree.leaf_colors(0).unwrap().len(), 1);
        assert_eq!(tree.color_at(v(0, 0, 0)), Some(color_of(v(0, 0, 0))));
        assert_eq!(tree.color_at(v(9, 0, 0)), None);
        assert_logical_pool_read(&tree);
    }

    #[test]
    fn colour_ops_are_inert_without_a_colour_store() {
        // set_color / set_voxel_colored on a plain (non-truecolor) tree touch
        // occupancy exactly like set_voxel and never manifest a colour store.
        let r = res(32);
        let v = VoxelCoord::new;
        let mut tree = SparseTree::build(&Empty { resolution: r });
        assert_eq!(
            tree.set_voxel_colored(v(0, 0, 0), true, 0x1234_5678),
            Edit::Topology
        );
        assert!(!tree.has_colors(), "colour dropped on a None-store tree");
        assert!(tree.is_occupied(v(0, 0, 0)));
        assert_eq!(tree.color_at(v(0, 0, 0)), None);
        assert_eq!(tree.set_color(v(0, 0, 0), 0xDEAD_BEEF), Edit::Unchanged);
        assert!(!tree.has_colors());
    }

    /// The promotion oracle (brush-editing 09 §promotion-oracle, the core
    /// half): after `promote_colors`, every occupied voxel's colour equals the
    /// map applied to its pre-promotion material, the dense store is dropped,
    /// occupancy is untouched, and the pool read stays logically consistent.
    #[test]
    fn promote_colors_bakes_the_material_map_and_drops_the_dense_store() {
        let r = res(32);
        let v = VoxelCoord::new;
        // A mixed scene: brick 0 carries real materials, brick 1 is global-0.
        let mut voxels = Vec::new();
        for x in 0..8u32 {
            for y in 0..4u32 {
                voxels.push((v(x, y, 0), u16::try_from(x % 3).unwrap()));
                voxels.push((v(x + 8, y, 0), 0u16));
            }
        }
        let mut tree = SparseTree::from_voxels(r, voxels.iter().copied());
        assert!(!tree.has_uniform_materials(), "brick 0 densified the store");
        let before_occ = tree.occupied_voxels();
        let generation = tree.topology_generation();

        // The map: gid 0 → a coordinate hash; real gids → a gid-keyed colour.
        let map = |c: VoxelCoord, gid: u16| -> u32 {
            if gid == 0 {
                0xFF00_0000 | (c.x * 73 + c.y * 31 + c.z)
            } else {
                0xFF10_0000 | u32::from(gid)
            }
        };
        tree.promote_colors(map);

        assert!(tree.has_colors());
        assert!(tree.has_uniform_materials(), "dense store must drop");
        assert_eq!(tree.occupied_voxels(), before_occ, "occupancy untouched");
        assert_eq!(tree.topology_generation(), generation, "no renumbering");
        for &(c, gid) in &voxels {
            assert_eq!(tree.color_at(c), Some(map(c, gid)), "{c:?}");
            assert_eq!(tree.material_at(c), 0, "materials read all-default now");
        }
        assert_logical_pool_read(&tree);

        // One-way: a second promotion is a no-op (colours already win).
        tree.promote_colors(|_, _| 0xDEAD_BEEF);
        assert_eq!(tree.color_at(v(0, 0, 0)), Some(map(v(0, 0, 0), 0)));
    }

    /// Logical equality for the undo path: same occupancy structure and
    /// materials byte-for-byte, same colours per slot by *content*. Page
    /// addresses are deliberately excluded — a restore may re-place a page, and
    /// the design's oracle is patched-page content, not address.
    fn assert_bricks_logically_equal(a: &SparseTree, b: &SparseTree) {
        assert_eq!(a.codes, b.codes, "brick sets differ");
        assert_eq!(a.leaves, b.leaves, "occupancy differs");
        assert_eq!(a.occupied_voxels(), b.occupied_voxels());
        for idx in 0..a.leaf_count() {
            assert_eq!(a.leaf_materials(idx), b.leaf_materials(idx), "slot {idx}");
            assert_eq!(a.leaf_colors(idx), b.leaf_colors(idx), "slot {idx}");
        }
        // Node levels rebuilt correctly (the once-at-the-end rebuild).
        assert_eq!(a.node_count(), b.node_count());
        for level in 0..a.nodes.len() {
            assert_eq!(a.nodes[level], b.nodes[level], "level {level}");
        }
    }

    #[test]
    fn replace_bricks_round_trips_a_heavy_edit_on_a_coloured_tree() {
        let r = res(32);
        let v = VoxelCoord::new;
        let mut map = BTreeMap::new();
        for x in 0..12u32 {
            for y in 0..4u32 {
                map.insert(v(x, y, 0), color_of(v(x, y, 0)));
            }
        }
        let mut tree = build_colored(r, &map);
        let original = tree.clone();

        // Capture every brick the coming "stroke" will touch — including one
        // that does not exist yet (its pre-image is None).
        let touched = [
            crate::morton::encode(0, 0, 0),
            crate::morton::encode(1, 0, 0),
            crate::morton::encode(3, 3, 3), // absent: pre-image None
        ];
        let pre: Vec<(u64, Option<BrickImage>)> = touched
            .iter()
            .map(|&code| (code, tree.brick_image(code)))
            .collect();
        assert!(pre[0].1.is_some() && pre[2].1.is_none());

        // The stroke: recolour, erase a whole brick, plant a new one.
        assert_ne!(tree.set_color(v(1, 1, 0), 0xFF12_3456), Edit::Unchanged);
        for x in 8..12u32 {
            for y in 0..4u32 {
                tree.set_voxel(v(x, y, 0), false); // empties brick (1,0,0)
            }
        }
        assert_eq!(
            tree.set_voxel_colored(v(30, 30, 30), true, 0xFFAB_CDEF),
            Edit::Topology
        );
        assert_ne!(tree.codes, original.codes);

        // Undo: restore the captured pre-images (the new brick restores to None).
        let deleted = crate::morton::encode(3, 3, 3);
        let undo: Vec<(u64, Option<BrickImage>)> = pre
            .iter()
            .map(|(code, img)| (*code, img.clone()))
            .chain([(deleted, None)])
            .collect();
        tree.replace_bricks(undo);
        assert_bricks_logically_equal(&tree, &original);
        assert_logical_pool_read(&tree);
    }

    #[test]
    fn replace_bricks_in_place_overwrite_keeps_the_generation() {
        // A restore that neither inserts nor removes a brick must not bump the
        // topology generation (no level rebuild, no renumbering).
        let r = res(32);
        let v = VoxelCoord::new;
        let mut map = BTreeMap::new();
        map.insert(v(0, 0, 0), 0xFF00_00FF);
        map.insert(v(1, 0, 0), 0xFF00_FF00);
        let mut tree = build_colored(r, &map);
        let img = tree.brick_image(0).expect("brick 0 exists");
        tree.set_color(v(0, 0, 0), 0xFFFF_FFFF);
        let generation = tree.topology_generation();
        tree.replace_bricks([(0u64, Some(img))]);
        assert_eq!(tree.topology_generation(), generation);
        assert_eq!(tree.color_at(v(0, 0, 0)), Some(0xFF00_00FF));
        assert_logical_pool_read(&tree);
    }

    #[test]
    fn replace_bricks_restores_dense_materials_and_stays_uniform_without_them() {
        let r = res(32);
        let v = VoxelCoord::new;
        // Dense arm: a brick with a nonzero material round-trips through
        // removal and reinsertion.
        let mut tree = SparseTree::from_voxels(r, [(v(1, 1, 1), 7u16), (v(9, 1, 1), 3u16)]);
        let code = crate::morton::encode(0, 0, 0);
        let img = tree.brick_image(code).expect("brick exists");
        tree.replace_bricks([(code, None)]);
        assert!(!tree.is_occupied(v(1, 1, 1)));
        assert_eq!(tree.leaf_count(), 1);
        tree.replace_bricks([(code, Some(img))]);
        assert!(tree.is_occupied(v(1, 1, 1)));
        assert_eq!(tree.material_at(v(1, 1, 1)), 7);
        assert_eq!(tree.material_at(v(9, 1, 1)), 3);

        // Uniform arm: material-less images never densify the store.
        let mut plain = SparseTree::from_voxels(r, [(v(1, 1, 1), 0u16)]);
        let img = plain.brick_image(code).expect("brick exists");
        assert!(plain.has_uniform_materials());
        plain.replace_bricks([(code, None), (code, Some(img))]);
        assert!(plain.has_uniform_materials(), "no grid should materialize");
        assert!(plain.is_occupied(v(1, 1, 1)));
    }

    #[test]
    fn replace_bricks_treats_an_empty_image_as_removal_and_reclasses_pages() {
        let r = res(32);
        let v = VoxelCoord::new;
        let mut map = BTreeMap::new();
        // 40 voxels in one brick: class 64 (two steps up from minimum).
        for i in 0..40u32 {
            map.insert(v(i % 8, (i / 8) % 8, i / 64), color_of(v(i % 8, i / 8, 0)));
        }
        let mut tree = build_colored(r, &map);
        let pages = tree.color_pages().unwrap();
        assert_eq!(pages.class_of(0), color_pool::class_for(40));

        // Restore the brick to a 1-voxel image: the page must reclass down.
        let mut one = LeafBrick::EMPTY;
        one.set_local(0, 0, 0);
        let img = BrickImage {
            occ: one,
            materials: None,
            colors: Some(vec![0xFF11_2233]),
        };
        tree.replace_bricks([(0u64, Some(img))]);
        let pages = tree.color_pages().unwrap();
        assert_eq!(pages.class_of(0), color_pool::class_for(1));
        assert_eq!(tree.color_at(v(0, 0, 0)), Some(0xFF11_2233));
        assert_logical_pool_read(&tree);

        // An image with empty occupancy removes the brick outright.
        tree.replace_bricks([(
            0u64,
            Some(BrickImage {
                occ: LeafBrick::EMPTY,
                materials: None,
                colors: Some(Vec::new()),
            }),
        )]);
        assert_eq!(tree.leaf_count(), 0);
    }
}
