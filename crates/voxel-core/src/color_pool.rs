//! The paged colour pool allocator (brush-editing Stage A1).
//!
//! An editable truecolor scene stores one `u32` (sRGB RGBA8, R low) **per
//! occupied voxel** in rank order, exactly as the build-once bake did — but
//! instead of packing every leaf's colours end-to-end (a prefix-sum layout that
//! must be rebuilt on any edit, see `docs/design/brush-editing/02`), each leaf's
//! colours live in a **page**: a contiguous run in a pool whose capacity is
//! rounded up to a size class. The slack a class buys is what makes editing
//! cheap — a leaf whose occupancy grows within its page rewrites only its own
//! page, and topology events rebuild a 4 B/leaf page table, never the pool.
//!
//! This module is the pure CPU allocator: size classes, per-chunk bump
//! watermarks, and per-class free lists. It hands out **entry offsets** (a page
//! is `[offset, offset + class)` in a flat entry space); the GPU maps that space
//! onto chunked storage buffers in Stage A2. The one hard geometric invariant is
//! **non-straddle**: a page never crosses a chunk boundary, so the GPU's
//! chunk-select (`offset / CHUNK_ENTRIES`) addresses a page with a single index.
//!
//! The pool is GPU-free and deterministic, so it is unit-tested in isolation
//! (Engineering Codex: Pure Core, Effectful Edges) — see the property tests at
//! the foot of this file.

/// Entries per size-class step. A page's capacity is always a multiple of this,
/// so per-leaf waste is at most `CLASS_STEP - 1` entries.
pub const CLASS_STEP: u32 = 32;

/// Largest page capacity — a fully-occupied `8³` leaf (512 voxels).
pub const MAX_CLASS: u32 = 512;

/// Number of size classes (`32, 64, …, 512`).
pub const N_CLASSES: usize = (MAX_CLASS / CLASS_STEP) as usize;

/// Entries per pool chunk. Mirrors `voxel_gpu::buffers::COLOR_PER_CHUNK`
/// (128 MiB / 4 B); a multiple of [`MAX_CLASS`] so a page never straddles a
/// chunk boundary. Stage A2 unifies the two definitions; A1 pins the value
/// (`CHUNK_ENTRIES % MAX_CLASS == 0`) with the compile-time assert below.
pub const CHUNK_ENTRIES: u64 = 33_554_432;

const _: () = assert!(
    CHUNK_ENTRIES.is_multiple_of(MAX_CLASS as u64),
    "CHUNK_ENTRIES must be a multiple of MAX_CLASS so pages never straddle a chunk boundary",
);

/// The GPU colour-pool chunk ceiling (`voxel_gpu::buffers::N_MAX_CHUNKS`),
/// mirrored here so the memory pin below can reference it. Stage A2 owns the
/// runtime cap; A1 pins that the size-class step keeps a Tokyo-class scene under
/// it.
const N_MAX_CHUNKS: u64 = 3;

// Brush-editing Stage A1 memory pin (docs/design/brush-editing/08). A Tokyo-class
// truecolor scene — LittlestTokyo at 2048³, ≈553,769 leaves and ≈75M occupied
// voxels — must fit the GPU pool's `N_MAX_CHUNKS`-chunk ceiling even at the worst
// per-leaf class waste (≤ CLASS_STEP-1 entries). This is the tightest number in
// the whole design (~8% slack), and it is exactly why the class step is 32 and
// not larger: a coarser step would blow the ceiling.
const _: () = assert!(
    75_000_000 + (CLASS_STEP as u64 - 1) * 553_769 <= N_MAX_CHUNKS * CHUNK_ENTRIES,
    "size-class waste would push a Tokyo-class scene past the colour-pool ceiling",
);

/// The page offset of a [`ColorLeaf`](crate::sparse) with no allocation yet
/// (an empty leaf, which a built structure never stores).
pub const PAGE_NONE: u32 = u32::MAX;

/// The page capacity, in entries, for a leaf holding `occupancy` occupied
/// voxels: `occupancy` rounded up to a multiple of [`CLASS_STEP`], clamped to
/// `[CLASS_STEP, MAX_CLASS]`. `occupancy == 0` yields `0` (no page).
#[must_use]
pub fn class_for(occupancy: u32) -> u32 {
    debug_assert!(
        occupancy <= MAX_CLASS,
        "occupancy {occupancy} exceeds an 8³ leaf"
    );
    if occupancy == 0 {
        return 0;
    }
    occupancy.div_ceil(CLASS_STEP) * CLASS_STEP
}

/// Packs a leaf's rank-order colours into a `class`-sized page image: the
/// colours followed by zero padding to the class capacity. The padding entries
/// are never read (a voxel's rank is always `< occupancy ≤ class`), so their
/// value is immaterial — zero keeps the image deterministic. This is the ≤ 2 KiB
/// buffer Stage A2 writes at the leaf's page offset.
///
/// # Panics
/// Panics if `colors.len() > class` (a page too small for its own colours — an
/// allocator/class-tracking bug).
#[must_use]
pub fn pack_page(colors: &[u32], class: u32) -> Vec<u32> {
    assert!(
        colors.len() <= class as usize,
        "pack_page: {} colours exceed class capacity {class}",
        colors.len()
    );
    let mut page = vec![0u32; class as usize];
    page[..colors.len()].copy_from_slice(colors);
    page
}

/// Free-list index for a (validated) class capacity: `32 → 0, 64 → 1, …`.
fn class_index(class: u32) -> usize {
    (class / CLASS_STEP - 1) as usize
}

/// The size-class page allocator over a flat entry space partitioned into fixed
/// chunks. Bump-allocates within a chunk (jumping to the next chunk rather than
/// letting a page straddle), and reuses freed pages exact-class-first.
#[derive(Debug, Clone)]
pub struct ColorPool {
    /// Entries per chunk — configurable so tests can force multi-chunk /
    /// straddle behaviour at small scale.
    chunk_entries: u64,
    /// Next never-allocated entry offset.
    watermark: u64,
    /// Freed pages by class (each holds page offsets of exactly that class).
    free: [Vec<u32>; N_CLASSES],
    /// Entries currently parked in the free lists (dead but not reclaimed).
    freed_entries: u64,
    /// Entries currently handed out (allocated and not freed).
    live_entries: u64,
}

impl Default for ColorPool {
    fn default() -> Self {
        Self::with_chunk_entries(CHUNK_ENTRIES)
    }
}

impl ColorPool {
    /// A pool with the production [`CHUNK_ENTRIES`] chunk size.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A pool with a caller-chosen chunk size. `chunk_entries` must be a
    /// multiple of [`MAX_CLASS`] (so no page straddles) and non-zero. Used by
    /// tests to exercise multi-chunk allocation at small scale.
    #[must_use]
    pub fn with_chunk_entries(chunk_entries: u64) -> Self {
        assert!(
            chunk_entries > 0 && chunk_entries.is_multiple_of(u64::from(MAX_CLASS)),
            "chunk_entries must be a non-zero multiple of MAX_CLASS"
        );
        Self {
            chunk_entries,
            watermark: 0,
            free: Default::default(),
            freed_entries: 0,
            live_entries: 0,
        }
    }

    /// Allocates a page of `class` entries and returns its entry offset. Reuses
    /// a freed same-class page when one exists, else bump-allocates — skipping to
    /// the next chunk rather than straddling a boundary. `class` must be a valid
    /// non-zero class (a multiple of [`CLASS_STEP`], `≤ MAX_CLASS`).
    pub fn alloc(&mut self, class: u32) -> u32 {
        debug_assert!(
            class > 0 && class <= MAX_CLASS && class.is_multiple_of(CLASS_STEP),
            "alloc: {class} is not a valid page class"
        );
        if let Some(page) = self.free[class_index(class)].pop() {
            self.freed_entries -= u64::from(class);
            self.live_entries += u64::from(class);
            return page;
        }
        // Bump within the current chunk; a page that would cross into the next
        // chunk instead starts the next chunk (the tail is wasted).
        let local = self.watermark % self.chunk_entries;
        if local + u64::from(class) > self.chunk_entries {
            self.watermark += self.chunk_entries - local;
        }
        let page = u32::try_from(self.watermark).expect("colour pool entry offset exceeds u32");
        self.watermark += u64::from(class);
        self.live_entries += u64::from(class);
        page
    }

    /// Returns a page of `class` entries to the pool. The caller must pass the
    /// same class the page was allocated with.
    pub fn free(&mut self, page: u32, class: u32) {
        debug_assert!(
            class > 0 && class <= MAX_CLASS && class.is_multiple_of(CLASS_STEP),
            "free: {class} is not a valid page class"
        );
        self.free[class_index(class)].push(page);
        self.freed_entries += u64::from(class);
        self.live_entries -= u64::from(class);
    }

    /// Entries currently allocated (excludes freed pages). The GPU pool need
    /// only be this large plus fragmentation; the [`watermark`](Self::watermark)
    /// is the actual high-water offset.
    #[must_use]
    pub fn live_entries(&self) -> u64 {
        self.live_entries
    }

    /// The high-water offset — one past the largest entry ever bump-allocated
    /// (including chunk-straddle padding). The GPU pool must span this.
    #[must_use]
    pub fn watermark(&self) -> u64 {
        self.watermark
    }

    /// Entries currently parked in the free lists (allocated space that is dead
    /// but not yet reclaimed). Drives [`needs_repack`](Self::needs_repack).
    #[must_use]
    pub fn freed_entries(&self) -> u64 {
        self.freed_entries
    }

    /// The configured chunk size in entries.
    #[must_use]
    pub fn chunk_entries(&self) -> u64 {
        self.chunk_entries
    }

    /// Whether fragmentation has grown enough to warrant a canonical repack
    /// (reassign every page in slot order, one pool re-upload — Stage A2's rare
    /// escape hatch). True when parked free space exceeds half the high-water
    /// mark: the pool is carrying substantial dead space it cannot bump past.
    #[must_use]
    pub fn needs_repack(&self) -> bool {
        self.watermark > 0 && self.freed_entries * 2 > self.watermark
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn class_for_rounds_up_in_steps_of_32() {
        assert_eq!(class_for(0), 0);
        assert_eq!(class_for(1), 32);
        assert_eq!(class_for(32), 32);
        assert_eq!(class_for(33), 64);
        assert_eq!(class_for(64), 64);
        assert_eq!(class_for(65), 96);
        assert_eq!(class_for(511), 512);
        assert_eq!(class_for(512), 512);
        // Waste is always < CLASS_STEP.
        for occ in 1..=MAX_CLASS {
            let waste = class_for(occ) - occ;
            assert!(
                waste < CLASS_STEP,
                "occ {occ}: waste {waste} ≥ {CLASS_STEP}"
            );
        }
    }

    #[test]
    fn pack_page_pads_to_class_with_zeros() {
        let colors = [0xAABB_CCDD, 0x1122_3344, 0x5566_7788];
        let page = pack_page(&colors, 32);
        assert_eq!(page.len(), 32);
        assert_eq!(&page[..3], &colors);
        assert!(page[3..].iter().all(|&w| w == 0), "padding must be zero");
    }

    #[test]
    #[should_panic = "exceed class capacity"]
    fn pack_page_rejects_colors_over_class() {
        let _ = pack_page(&[0; 40], 32);
    }

    #[test]
    fn bump_allocation_is_sequential_and_dense_within_a_chunk() {
        // A fresh pool with no frees hands out class-sized blocks end to end —
        // the canonical assignment install_colors relies on.
        let mut pool = ColorPool::new();
        assert_eq!(pool.alloc(32), 0);
        assert_eq!(pool.alloc(64), 32);
        assert_eq!(pool.alloc(32), 96);
        assert_eq!(pool.watermark(), 128);
        assert_eq!(pool.live_entries(), 128);
    }

    #[test]
    fn a_page_never_straddles_a_chunk_boundary() {
        // Tiny chunk of 512 entries. After 480 used (15×32), a class-64 page
        // would cross into chunk 1, so it must start at 512, wasting 32 entries.
        let mut pool = ColorPool::with_chunk_entries(512);
        for _ in 0..15 {
            pool.alloc(32);
        }
        assert_eq!(pool.watermark(), 480);
        let page = pool.alloc(64);
        assert_eq!(page, 512, "class-64 page must jump past the chunk boundary");
        assert_eq!(page % 512 + 64, 64, "page fits wholly inside chunk 1");
    }

    #[test]
    fn free_then_alloc_reuses_the_exact_page() {
        let mut pool = ColorPool::new();
        let a = pool.alloc(96);
        let b = pool.alloc(96);
        pool.free(a, 96);
        assert_eq!(pool.freed_entries(), 96);
        // Same class → the freed page comes back, watermark does not grow.
        let wm = pool.watermark();
        let c = pool.alloc(96);
        assert_eq!(c, a, "exact-class reuse");
        assert_eq!(pool.watermark(), wm, "reuse must not bump the watermark");
        assert_eq!(pool.freed_entries(), 0);
        assert_ne!(b, a);
    }

    #[test]
    fn needs_repack_flips_under_heavy_fragmentation() {
        let mut pool = ColorPool::new();
        let pages: Vec<u32> = (0..10).map(|_| pool.alloc(512)).collect();
        assert!(!pool.needs_repack(), "no frees yet");
        // Free most of them: parked free space now dominates the high-water mark.
        for &p in &pages[..6] {
            pool.free(p, 512);
        }
        assert!(pool.needs_repack(), "6/10 pages dead → repack");
    }

    fn splitmix64(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// The pool's sharp oracle: under a long random alloc/free script with a
    /// small multi-chunk pool, the live pages stay pairwise disjoint, never
    /// straddle a chunk boundary, and `live_entries` exactly conserves the sum of
    /// live page sizes. A single off-by-one in the bump/reuse math trips one of
    /// these on the way through.
    #[test]
    fn live_pages_are_disjoint_non_straddling_and_conserved() {
        let chunk = 1024u64; // 2 chunks worth of headroom exercised below
        let mut pool = ColorPool::with_chunk_entries(chunk);
        let mut live: BTreeMap<u32, u32> = BTreeMap::new(); // page → class
        let mut state = 0xC010_1234_5678_9ABCu64;

        for _ in 0..8000 {
            let roll = splitmix64(&mut state) % 3;
            if roll == 0 && !live.is_empty() {
                // Free a random live page.
                let keys: Vec<u32> = live.keys().copied().collect();
                let pick = usize::try_from(splitmix64(&mut state) % keys.len() as u64).unwrap();
                let victim = keys[pick];
                let class = live.remove(&victim).unwrap();
                pool.free(victim, class);
            } else {
                // Allocate a random class.
                let ci = u32::try_from(splitmix64(&mut state) % N_CLASSES as u64).unwrap();
                let class = (ci + 1) * CLASS_STEP;
                let page = pool.alloc(class);
                let lo = u64::from(page);
                let hi = lo + u64::from(class);
                // Non-straddle: the page fits wholly within one chunk.
                assert_eq!(
                    lo / chunk,
                    (hi - 1) / chunk,
                    "page {page}(+{class}) straddles a chunk"
                );
                // Disjoint from every other live page.
                for (&p, &c) in &live {
                    let (a, b) = (u64::from(p), u64::from(p) + u64::from(c));
                    assert!(
                        hi <= a || lo >= b,
                        "page {page}(+{class}) overlaps live {p}(+{c})"
                    );
                }
                live.insert(page, class);
            }
            // Conservation: live_entries == Σ live page classes.
            let sum: u64 = live.values().map(|&c| u64::from(c)).sum();
            assert_eq!(
                pool.live_entries(),
                sum,
                "live_entries drifted from the live set"
            );
        }
    }
}
