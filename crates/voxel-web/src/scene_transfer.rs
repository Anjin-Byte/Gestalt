//! The cross-worker scene blob: one flat byte buffer carrying a built scene
//! (tree + School-B structure + material table) between the IO worker and the
//! render engine (`docs/design/web-frontend-api.md` §5, stage 7).
//!
//! One buffer means one `postMessage` transferable — a zero-copy move between
//! JS contexts. This is an **internal protocol**, not a file format: both ends
//! ship in the same wasm binary, so the version bump rule is simply "change
//! the layout, bump `VERSION`, both sides rebuild together".
//!
//! Layout (all little-endian), after an 8-field × u32 header:
//! `codes (u64×L) · leaf words32 (u32×16L) · [materials (u16×512L)] ·
//! table (u32×T) · colors (u32×C)`. The dense per-voxel material section is
//! flagged: a scene whose table is missing-only (fixtures, plain noise) has
//! every material global-0, so ~1 KiB/leaf of zeros would ride every copy of
//! the blob — the flag drops the section instead. The colour **page table is not
//! carried**: deserialization installs the colours into the tree's editable
//! paged store ([`SparseTree::install_colors`], brush-editing Stage A2), and
//! [`SchoolBBuffer::from_sparse`] then derives the per-leaf page offsets and
//! transparency bits from it. Both directions walk leaf-slot × intra-brick-Morton
//! order, so a serialize→deserialize→serialize round-trip is byte-identical.

use voxel_core::LEAF_VOXELS;
use voxel_core::{
    LeafBrick, MaterialTable, Progress, Resolution, SchoolBBuffer, SparseTree, morton,
};

/// `b"VXSC"` — voxel scene.
const MAGIC: u32 = u32::from_le_bytes(*b"VXSC");
const VERSION: u32 = 2;
/// Header: magic, version, `n`, `leaf_count`, `table_len`, `color_len`,
/// `models_total`, flags.
const HEADER_WORDS: usize = 8;
/// Header flag: the dense per-voxel material section is present.
const FLAG_DENSE_MATERIALS: u32 = 1;

/// Why a scene blob could not be decoded.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TransferError {
    /// Not a scene blob, or a layout this build does not speak.
    #[error("scene blob: {0}")]
    Malformed(String),
}

// --- Peak-estimate cost model ---------------------------------------------
//
// Per leaf, always resident while the tree, the School-B structure, and this
// blob are all alive at once (the install/build peak): brick data duplicated
// in `tree.leaves` and `structure.leaves` (64B × 2), `leaf_bounds` (4B), the
// School-B palette slot (`STRIDE_W` = 73 `u32` words = 292B), and the blob's
// codes + leaf-words32 section (8B + 64B). 64·2+4+292+72 = 496, rounded up to
// 512 for allocator/alignment slack.
const BASE_BYTES_PER_LEAF: u64 = 512;
/// Extra per leaf *only* when the tree actually assigns materials
/// (`!SparseTree::has_uniform_materials`): the tree's dense `Box<[u16;512]>`
/// grid (1024B) plus the blob's dense-materials section (1024B). A
/// never-coloured scene (fixtures, plain noise, an untouched brush session)
/// pays none of this — see [`voxel_core::SparseTree`]'s `Uniform`/`Dense` split.
const DENSE_MATERIALS_BYTES_PER_LEAF: u64 = 2048;
/// Extra *per occupied voxel* — not per leaf — only when truecolor baked:
/// `leaf_color` (4B) plus the blob's colour section (4B). Scaled by the exact
/// occupied-voxel count rather than assuming every leaf is fully packed.
const TRUECOLOR_BYTES_PER_VOXEL: u64 = 8;

/// The web build's scene budget: bounds one build's transient peak against
/// wasm32's *hard, universal* ceiling — 4 GiB (65536 pages), true on every
/// engine regardless of browser — not against any one browser's tab-memory
/// heuristic. That distinction matters in practice: a 2048³ truecolor
/// `LittlestTokyo` import (~553k leaves, real per-triangle materials, ~1.65 GiB
/// estimated) worked fine before this guard existed, and an earlier version
/// of this constant — reasoned from Safari/WebKit's specific, much lower
/// jetsam threshold (real-world reports place it as low as ~1.5 GiB *total*
/// for a tab) — wrongly rejected it. Only a build that would *actually* fail
/// to allocate, on any engine, belongs behind a hard pre-build gate; leaving
/// 25% of the address space as margin covers wasm module/allocator overhead
/// and the parts of a build this estimate does not itemize.
///
/// The Safari-specific concern (a real, much tighter ceiling on that one
/// engine) is real but is not this gate's job: it is served by the *observed*
/// wasm-heap gauge in the HUD and by `IoClient`'s idle-worker recycling
/// (`web/src/io.ts`, `IO_RECYCLE_HEAP_BYTES`) — both act on actual measured
/// memory after the fact, rather than refusing a build a capable browser
/// could have completed.
const WEB_SCENE_BUDGET_BYTES: u64 = 3 * (1 << 30); // 3 GiB — 75% of the wasm32 hard ceiling

// Regression pins (compile-time — every value here is a `const`, so a future
// edit that drifts the budget back toward a browser-specific heuristic fails
// the *build*, not just a test run). An earlier version of this constant was
// reasoned from Safari/WebKit's specific tab-jetsam ceiling (~1.5 GiB) rather
// than wasm32's actual 4 GiB address-space wall, and wrongly rejected a real,
// working 2048³ truecolor `LittlestTokyo` import: ~553k leaves, dense
// materials (no truecolor — the cheaper of the two extra terms), ~1.65 GiB
// estimated.
const _: () = assert!(WEB_SCENE_BUDGET_BYTES > 1536 * (1u64 << 20));
const _: () = assert!(
    553_769_u64 * (BASE_BYTES_PER_LEAF + DENSE_MATERIALS_BYTES_PER_LEAF) < WEB_SCENE_BUDGET_BYTES
);
// Brush-editing Stage A1 (docs/design/brush-editing/08): the same Tokyo-class
// scene as an *editable truecolor* import — ≈553,769 leaves and ≈75M occupied
// voxels — also fits. The paged colour store moves the per-voxel colour bytes
// from the structure to the tree one-for-one, so `TRUECOLOR_BYTES_PER_VOXEL` is
// unchanged; the per-leaf page word and colour-`Vec` metadata are a few dozen
// bytes/leaf, absorbed by `BASE_BYTES_PER_LEAF`'s rounding slack and the budget's
// 25% margin.
const _: () = assert!(
    553_769_u64 * BASE_BYTES_PER_LEAF + 75_000_000_u64 * TRUECOLOR_BYTES_PER_VOXEL
        < WEB_SCENE_BUDGET_BYTES
);
// Brush-editing Stage B (docs/design/brush-editing/05): the undo ring's byte
// budget rides alongside the scene, so the two must fit the wasm32 4 GiB
// address-space wall *together* — with headroom left of the 25% margin for
// wasm module/allocator overhead.
const _: () =
    assert!(WEB_SCENE_BUDGET_BYTES + crate::undo::UNDO_BUDGET_BYTES as u64 <= 3328 * (1u64 << 20));

/// A scene too large for the web build's memory budget.
#[derive(Debug, thiserror::Error)]
#[error(
    "scene too large for the web build: {leaf_count} leaf bricks ≈ {est_mib:.0} MiB peak \
     (budget {budget_mib:.0} MiB); lower the resolution{truecolor_hint}",
    est_mib = *est_bytes as f64 / f64::from(1u32 << 20),
    budget_mib = *budget_bytes as f64 / f64::from(1u32 << 20),
    truecolor_hint = if *truecolor_dominant { ", or disable truecolor" } else { "" },
)]
pub(crate) struct SceneBudgetError {
    /// Leaf bricks in the built scene.
    leaf_count: usize,
    /// The estimate that tripped the budget.
    est_bytes: u64,
    /// The budget it tripped (the production constant in real use; tests vary
    /// it so the boundary is checkable on small trees).
    budget_bytes: u64,
    /// Whether the truecolor bake was the larger contributor — worth
    /// surfacing since it is a knob the shell exposes independent of resolution.
    truecolor_dominant: bool,
}

/// The peak-bytes estimate for `(tree, structure)` and whether the truecolor
/// bake (rather than dense materials) is the larger contributor — pure and
/// tree-size-independent in cost, so tests exercise it on trivial trees
/// instead of constructing production-scale ones just to hit the arithmetic.
fn estimate_scene_bytes(tree: &SparseTree, structure: &SchoolBBuffer) -> (u64, bool) {
    let leaf_count = tree.leaf_count() as u64;
    let base = leaf_count * BASE_BYTES_PER_LEAF;
    let dense = if tree.has_uniform_materials() {
        0
    } else {
        leaf_count * DENSE_MATERIALS_BYTES_PER_LEAF
    };
    let truecolor = if structure.has_leaf_color() {
        tree.occupied_voxels() * TRUECOLOR_BYTES_PER_VOXEL
    } else {
        0
    };
    (base + dense + truecolor, truecolor > dense)
}

/// [`check_scene_budget`] against an explicit `budget` — the gate logic tests
/// exercise directly (a small tree can trip a small budget; it can never
/// naturally reach [`WEB_SCENE_BUDGET_BYTES`]).
fn check_scene_budget_against(
    tree: &SparseTree,
    structure: &SchoolBBuffer,
    budget_bytes: u64,
) -> Result<(), SceneBudgetError> {
    let (est_bytes, truecolor_dominant) = estimate_scene_bytes(tree, structure);
    if est_bytes > budget_bytes {
        return Err(SceneBudgetError {
            leaf_count: tree.leaf_count(),
            est_bytes,
            budget_bytes,
            truecolor_dominant,
        });
    }
    Ok(())
}

/// Guards a built scene against wasm32's *hard* address-space wall before it
/// is serialized and shipped to the render worker
/// (`docs/design/web-frontend-api.md` §8 — a typed "too large" beats an
/// out-of-memory panic on any engine). This is deliberately not tuned to any
/// one browser's soft tab-memory heuristic — see [`WEB_SCENE_BUDGET_BYTES`].
/// Uses the *actual* tree/structure rather than a flat per-leaf guess: a
/// never-coloured scene (fixtures, noise) is ~13× cheaper per leaf than a
/// densely-materialed, fully-baked truecolor one, and this estimate reflects
/// that exactly instead of over- or under-shooting either case.
pub(crate) fn check_scene_budget(
    tree: &SparseTree,
    structure: &SchoolBBuffer,
) -> Result<(), SceneBudgetError> {
    check_scene_budget_against(tree, structure, WEB_SCENE_BUDGET_BYTES)
}

/// A decoded scene plus the transfer metadata the label logic wants.
pub(crate) struct TransferredScene {
    pub(crate) tree: SparseTree,
    pub(crate) structure: SchoolBBuffer,
    pub(crate) table: MaterialTable,
    /// Source-file model count (`.vox`/`.cvox` imports; 1 otherwise).
    pub(crate) models_total: u32,
}

/// Flattens a scene into the transfer blob. Meters `progress` by **bytes
/// written** against the exact precomputed total — for a big scene the blob is
/// hundreds of MB of copying, a real wait worth a real fraction (the shell's
/// `pack` phase). Pass [`Progress::none`] where progress is not observed.
pub(crate) fn serialize_scene(
    tree: &SparseTree,
    structure: &SchoolBBuffer,
    table: &MaterialTable,
    models_total: u32,
    progress: &mut Progress,
) -> Vec<u8> {
    let leaf_count = tree.leaf_count();
    let leaves = structure.leaves();
    debug_assert_eq!(leaves.len(), leaf_count, "tree/structure leaf parity");
    let table_words = table.words();
    // Colours come from the editable tree store when present (an installed
    // truecolor scene), else the build-once structure bake (a freshly produced
    // scene). Both walk leaf-slot then intra-brick-Morton (rank) order, so the
    // emitted bytes are identical — the blob wire format is unchanged (no VERSION
    // bump), and the round-trip stays byte-exact.
    let tree_colors: Vec<u32>;
    let colors: &[u32] = if tree.has_colors() {
        tree_colors = (0..leaf_count)
            .flat_map(|i| tree.leaf_colors(i).expect("has_colors").iter().copied())
            .collect();
        &tree_colors
    } else {
        structure.leaf_color_words()
    };
    // A missing-only table means every voxel is global-0: the dense material
    // section would be all zeros, so it is dropped (~1 KiB/leaf per copy).
    let dense_materials = table_words.len() > 1;

    let bytes = HEADER_WORDS * 4
        + leaf_count * 8
        + leaf_count * 16 * 4
        + if dense_materials {
            leaf_count * LEAF_VOXELS * 2
        } else {
            0
        }
        + table_words.len() * 4
        + colors.len() * 4;
    let mut meter = progress.meter(bytes as u64);
    let mut out = Vec::with_capacity(bytes);

    for word in [
        MAGIC,
        VERSION,
        tree.resolution().voxels_per_axis(),
        u32::try_from(leaf_count).expect("leaf count fits u32"),
        u32::try_from(table_words.len()).expect("table fits u32"),
        u32::try_from(colors.len()).expect("colour count fits u32"),
        models_total,
        if dense_materials {
            FLAG_DENSE_MATERIALS
        } else {
            0
        },
    ] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    meter.add(HEADER_WORDS as u64 * 4);
    for idx in 0..leaf_count {
        let origin = tree.leaf_origin(idx);
        let code = morton::encode(origin.x >> 3, origin.y >> 3, origin.z >> 3);
        out.extend_from_slice(&code.to_le_bytes());
        meter.add(8);
    }
    for leaf in leaves {
        for word in leaf.words32() {
            out.extend_from_slice(&word.to_le_bytes());
        }
        meter.add(16 * 4);
    }
    if dense_materials {
        for idx in 0..leaf_count {
            for &gid in tree.leaf_materials(idx) {
                out.extend_from_slice(&gid.to_le_bytes());
            }
            meter.add(LEAF_VOXELS as u64 * 2);
        }
    }
    for &word in table_words {
        out.extend_from_slice(&word.to_le_bytes());
    }
    meter.add(table_words.len() as u64 * 4);
    for &word in colors {
        out.extend_from_slice(&word.to_le_bytes());
        meter.add(4);
    }
    debug_assert_eq!(out.len(), bytes, "layout arithmetic must match the writes");
    out
}

/// Rebuilds a scene from the transfer blob, validating every length so a
/// corrupt buffer is a typed error rather than a broken tree.
#[allow(clippy::too_many_lines)] // a linear header→section→assemble decode; splitting hides the layout
pub(crate) fn deserialize_scene(bytes: &[u8]) -> Result<TransferredScene, TransferError> {
    let word = |i: usize| -> Result<u32, TransferError> {
        bytes
            .get(i * 4..i * 4 + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or_else(|| TransferError::Malformed("truncated header".to_string()))
    };
    if word(0)? != MAGIC {
        return Err(TransferError::Malformed("bad magic".to_string()));
    }
    if word(1)? != VERSION {
        return Err(TransferError::Malformed(format!(
            "version {} (this build speaks {VERSION})",
            word(1)?
        )));
    }
    let n = word(2)?;
    let leaf_count = word(3)? as usize;
    let table_len = word(4)? as usize;
    let color_len = word(5)? as usize;
    let models_total = word(6)?;
    let flags = word(7)?;
    if flags & !FLAG_DENSE_MATERIALS != 0 {
        return Err(TransferError::Malformed(format!(
            "unknown flags {flags:#x} (this build speaks {FLAG_DENSE_MATERIALS:#x})"
        )));
    }
    let dense_materials = flags & FLAG_DENSE_MATERIALS != 0;

    let resolution =
        Resolution::new(n).map_err(|e| TransferError::Malformed(format!("resolution: {e}")))?;
    let expected = HEADER_WORDS * 4
        + leaf_count * 8
        + leaf_count * 16 * 4
        + if dense_materials {
            leaf_count * LEAF_VOXELS * 2
        } else {
            0
        }
        + table_len * 4
        + color_len * 4;
    if bytes.len() != expected {
        return Err(TransferError::Malformed(format!(
            "length {} does not match the declared counts (expected {expected})",
            bytes.len()
        )));
    }

    let mut at = HEADER_WORDS * 4;
    let mut take = |len: usize| {
        let s = &bytes[at..at + len];
        at += len;
        s
    };

    let codes: Vec<u64> = take(leaf_count * 8)
        .chunks_exact(8)
        .map(|c| u64::from_le_bytes(c.try_into().expect("8-byte chunk")))
        .collect();
    if !codes.windows(2).all(|w| w[0] < w[1]) {
        return Err(TransferError::Malformed(
            "leaf codes not strictly ascending".to_string(),
        ));
    }
    let leaves: Vec<LeafBrick> = take(leaf_count * 16 * 4)
        .chunks_exact(16 * 4)
        .map(|leaf| {
            let mut words = [0u32; 16];
            for (w, c) in words.iter_mut().zip(leaf.chunks_exact(4)) {
                *w = u32::from_le_bytes(c.try_into().expect("4-byte chunk"));
            }
            LeafBrick::from_words32(words)
        })
        .collect();
    // A flag-less blob (missing-only table: every voxel global-0) deserializes
    // straight into the tree's storage-free uniform material form.
    let materials =
        dense_materials.then(|| read_dense_materials(take(leaf_count * LEAF_VOXELS * 2)));
    let table_words = read_u32s(take(table_len * 4));
    let colors = read_u32s(take(color_len * 4));

    let mut tree = SparseTree::from_parts(resolution, codes, leaves, materials);
    // Install colours into the *tree*'s editable paged store (Stage A2) — not the
    // build-once structure bake — so `from_sparse` below derives the page table +
    // transparency bits from it and the render engine gets the paged renderer.
    if color_len > 0 {
        let occupied = tree.occupied_voxels();
        if u64::try_from(colors.len()).expect("usize fits u64") != occupied {
            return Err(TransferError::Malformed(format!(
                "colour count {} does not match {occupied} occupied voxels",
                colors.len()
            )));
        }
        tree.install_colors(colors.into_iter());
    }
    let structure = SchoolBBuffer::from_sparse(&tree);

    if table_words.len() > u16::MAX as usize + 1 {
        return Err(TransferError::Malformed(
            "material table over the 65536-slot ceiling".to_string(),
        ));
    }
    let mut table = MaterialTable::missing_only();
    for &color in table_words.get(1..).unwrap_or(&[]) {
        table
            .push(color)
            .map_err(|e| TransferError::Malformed(e.to_string()))?;
    }

    Ok(TransferredScene {
        tree,
        structure,
        table,
        models_total,
    })
}

/// Reads the dense per-voxel material section (one `u16` grid per leaf).
#[allow(clippy::vec_box)] // the box-per-leaf shape is `SparseTree::from_parts`'s API
fn read_dense_materials(bytes: &[u8]) -> Vec<Box<[u16; LEAF_VOXELS]>> {
    bytes
        .chunks_exact(LEAF_VOXELS * 2)
        .map(|leaf| {
            let mut grid = Box::new([0u16; LEAF_VOXELS]);
            for (m, c) in grid.iter_mut().zip(leaf.chunks_exact(2)) {
                *m = u16::from_le_bytes([c[0], c[1]]);
            }
            grid
        })
        .collect()
}

/// Reads a packed little-endian `u32` section.
fn read_u32s(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().expect("4-byte chunk")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxel_core::VoxelCoord;

    fn scene() -> (SparseTree, SchoolBBuffer, MaterialTable) {
        let mut table = MaterialTable::missing_only();
        let a = table.push(0xFF11_2233).expect("push");
        let b = table.push(0xFF44_5566).expect("push");
        let resolution = Resolution::new(32).expect("legal");
        let tree = SparseTree::from_voxels(
            resolution,
            [
                (VoxelCoord::new(1, 2, 3), a),
                (VoxelCoord::new(1, 3, 3), b),
                (VoxelCoord::new(30, 2, 9), a),
                (VoxelCoord::new(0, 31, 31), b),
            ],
        );
        let structure = SchoolBBuffer::from_sparse(&tree);
        (tree, structure, table)
    }

    #[test]
    fn serialize_meters_by_bytes_to_the_exact_blob_length() {
        // The pack phase's contract: the meter's total is the blob's exact
        // byte length, emissions are monotone, and the terminal emission
        // reaches the total.
        let (tree, structure, table) = scene();
        let mut events: Vec<(u64, u64)> = Vec::new();
        let mut sink = |done, total| events.push((done, total));
        let blob = serialize_scene(&tree, &structure, &table, 1, &mut Progress::new(&mut sink));
        let total = blob.len() as u64;
        assert_eq!(
            events.first(),
            Some(&(0, total)),
            "meter opens at (0, blob len)"
        );
        assert_eq!(
            events.last(),
            Some(&(total, total)),
            "meter finishes at the total"
        );
        assert!(events.windows(2).all(|w| w[0].0 <= w[1].0), "monotone");
        assert!(events.iter().all(|&(_, t)| t == total), "stable total");
    }

    #[test]
    fn palette_scene_round_trips() {
        let (tree, structure, table) = scene();
        let blob = serialize_scene(&tree, &structure, &table, 3, &mut Progress::none());
        let got = deserialize_scene(&blob).expect("decode");

        assert_eq!(got.models_total, 3);
        assert_eq!(got.tree.occupied_voxels(), tree.occupied_voxels());
        assert_eq!(got.structure.nodes(), structure.nodes());
        assert_eq!(got.structure.leaves(), structure.leaves());
        assert_eq!(got.table.words(), table.words());
        assert_eq!(
            got.tree.material_at(VoxelCoord::new(30, 2, 9)),
            tree.material_at(VoxelCoord::new(30, 2, 9))
        );
        assert!(!got.structure.has_leaf_color());
    }

    /// The Stage-D promotion round-trip (09 §promotion-oracle): a palette
    /// scene promoted in place serializes as a truecolor blob whose second
    /// generation is byte-identical — the wire format needs no VERSION bump
    /// for promoted scenes either.
    #[test]
    fn promoted_scene_blob_round_trips_byte_identical() {
        let (mut tree, _structure, table) = scene();
        let n = tree.resolution().voxels_per_axis();
        tree.promote_colors(|c, gid| crate::edit::promotion_color(&table, n, c, gid));
        assert!(tree.has_colors());
        let structure = SchoolBBuffer::from_sparse(&tree);

        let first = serialize_scene(&tree, &structure, &table, 1, &mut Progress::none());
        let got = deserialize_scene(&first).expect("decode");
        let second = serialize_scene(
            &got.tree,
            &got.structure,
            &got.table,
            got.models_total,
            &mut Progress::none(),
        );
        assert_eq!(
            first, second,
            "second-generation blob must be byte-identical"
        );
        assert_eq!(
            got.tree.color_at(VoxelCoord::new(1, 2, 3)),
            Some(table.color(1)),
            "promoted palette colour survives the wire"
        );
    }

    #[test]
    fn truecolor_scene_round_trips_with_transparency_bits() {
        let (tree, mut structure, table) = scene();
        // Deterministic colours; one voxel semi-transparent to exercise the
        // transparency re-derivation on the far side.
        let byte = |v: u32| u8::try_from(v & 0xff).unwrap();
        let color_of = |c: VoxelCoord| {
            let alpha = if c == VoxelCoord::new(1, 3, 3) {
                128
            } else {
                255
            };
            [byte(c.x), byte(c.y), byte(c.z), alpha]
        };
        structure.assemble_leaf_color(&tree, color_of);
        assert!(structure.has_transparency());

        let blob = serialize_scene(&tree, &structure, &table, 1, &mut Progress::none());
        let got = deserialize_scene(&blob).expect("decode");

        // Colours now live in the tree's editable paged store (Stage A2); the
        // structure carries the *derived* page table + transparency bits rather
        // than the build-once `leaf_color` bake.
        assert!(got.tree.has_colors());
        assert!(
            !got.structure.has_leaf_color(),
            "no build-once bake on the render side"
        );
        assert!(
            got.structure.has_transparency(),
            "from_sparse must derive transparency from the tree store"
        );
        // The transparency bits land in exactly the leaf_bounds words the producer
        // path set (both OR TRANSPARENCY_BIT for the same transparent leaves).
        assert_eq!(
            got.structure.leaf_bounds_words(),
            structure.leaf_bounds_words(),
            "derived transparency bits diverged from the assembler's"
        );
        // Every occupied voxel's colour round-trips through the tree store.
        for c in [
            VoxelCoord::new(1, 2, 3),
            VoxelCoord::new(1, 3, 3),
            VoxelCoord::new(30, 2, 9),
            VoxelCoord::new(0, 31, 31),
        ] {
            assert_eq!(
                got.tree.color_at(c),
                Some(u32::from_le_bytes(color_of(c))),
                "colour at {c:?}"
            );
        }
        // The second-generation blob is byte-identical — the wire format did not
        // change (no VERSION bump), so a re-export of an installed scene matches.
        let reblob = serialize_scene(
            &got.tree,
            &got.structure,
            &got.table,
            1,
            &mut Progress::none(),
        );
        assert_eq!(
            reblob, blob,
            "second-generation blob must be byte-identical"
        );
    }

    #[test]
    fn corrupt_blobs_are_typed_errors() {
        let (tree, structure, table) = scene();
        let blob = serialize_scene(&tree, &structure, &table, 1, &mut Progress::none());

        assert!(
            deserialize_scene(&blob[..blob.len() - 1]).is_err(),
            "truncated"
        );
        assert!(deserialize_scene(b"garbage").is_err(), "not a blob");
        let mut wrong_version = blob.clone();
        wrong_version[4] = 9;
        assert!(deserialize_scene(&wrong_version).is_err(), "future version");
        let mut wrong_flags = blob;
        wrong_flags[7 * 4] |= 0x80;
        assert!(deserialize_scene(&wrong_flags).is_err(), "unknown flags");
    }

    #[test]
    fn missing_only_scene_drops_the_dense_material_section() {
        // A fixture-style scene: occupancy but no palette beyond global-0.
        let resolution = Resolution::new(32).expect("legal");
        let tree = SparseTree::from_voxels(
            resolution,
            [
                (VoxelCoord::new(1, 2, 3), 0),
                (VoxelCoord::new(30, 2, 9), 0),
                (VoxelCoord::new(0, 31, 31), 0),
            ],
        );
        let structure = SchoolBBuffer::from_sparse(&tree);
        let table = MaterialTable::missing_only();

        let blob = serialize_scene(&tree, &structure, &table, 1, &mut Progress::none());
        // ~1 KiB/leaf of zeros must NOT ride the blob (the whole point).
        let leaf_count = tree.leaf_count();
        let with_materials = HEADER_WORDS * 4
            + leaf_count * (8 + 16 * 4 + LEAF_VOXELS * 2)
            + table.words().len() * 4;
        assert_eq!(blob.len(), with_materials - leaf_count * LEAF_VOXELS * 2);

        let got = deserialize_scene(&blob).expect("decode");
        assert_eq!(got.tree.occupied_voxels(), tree.occupied_voxels());
        assert_eq!(got.structure.nodes(), structure.nodes());
        assert_eq!(got.structure.leaves(), structure.leaves());
        assert_eq!(got.tree.material_at(VoxelCoord::new(1, 2, 3)), 0);
        assert_eq!(got.table.words(), table.words());
        // The receive side must land in the storage-free uniform form — not
        // re-materialize 1 KiB/leaf of zeros it just avoided shipping.
        assert!(got.tree.has_uniform_materials());
    }

    /// A tree of `leaves` bricks at ascending synthetic codes, via the
    /// scan-free `from_parts` path — fast regardless of `leaves`, so tests can
    /// afford real (if not production-scale) leaf counts. `full` occupies
    /// every voxel in every brick (the worst case the truecolor term assumes);
    /// otherwise each brick holds a single voxel.
    fn synthetic_tree(leaves: usize, full: bool, materials: Option<u16>) -> SparseTree {
        let resolution = Resolution::new(2048).expect("legal");
        let bpa = resolution.voxels_per_axis() / 8; // bricks per axis = 256
        assert!(
            leaves <= bpa as usize * bpa as usize,
            "fits a 2D sheet of bricks at this res"
        );
        let brick = if full {
            let mut b = voxel_core::LeafBrick::EMPTY;
            for z in 0..8u32 {
                for y in 0..8u32 {
                    for x in 0..8u32 {
                        b.set_local(x, y, z);
                    }
                }
            }
            b
        } else {
            let mut b = voxel_core::LeafBrick::EMPTY;
            b.set_local(0, 0, 0);
            b
        };
        let codes: Vec<u64> = (0..leaves as u32)
            .map(|i| morton::encode(i % bpa, i / bpa, 0))
            .collect();
        let mut sorted = codes.clone();
        sorted.sort_unstable();
        let leaves_vec = vec![brick; leaves];
        let materials = materials.map(|gid| {
            let grid: Box<[u16; LEAF_VOXELS]> = Box::new([gid; LEAF_VOXELS]);
            vec![grid; leaves]
        });
        SparseTree::from_parts(resolution, sorted, leaves_vec, materials)
    }

    #[test]
    fn estimate_matches_the_documented_cost_model_per_combination() {
        // Small, deliberately mixed trees (some full bricks, some single-voxel)
        // so the truecolor term's per-occupied-voxel scaling is exercised, not
        // just its per-leaf shape.
        let uniform_no_color = synthetic_tree(3, false, None);
        let structure = SchoolBBuffer::from_sparse(&uniform_no_color);
        assert!(uniform_no_color.has_uniform_materials());
        assert!(!structure.has_leaf_color());
        let (est, dominant) = estimate_scene_bytes(&uniform_no_color, &structure);
        assert_eq!(est, 3 * BASE_BYTES_PER_LEAF);
        assert!(!dominant);

        let dense_no_color = synthetic_tree(3, false, Some(7));
        let structure = SchoolBBuffer::from_sparse(&dense_no_color);
        assert!(!dense_no_color.has_uniform_materials());
        let (est, dominant) = estimate_scene_bytes(&dense_no_color, &structure);
        assert_eq!(
            est,
            3 * (BASE_BYTES_PER_LEAF + DENSE_MATERIALS_BYTES_PER_LEAF)
        );
        assert!(!dominant, "dense materials present but truecolor is not");

        let uniform_with_color = synthetic_tree(2, true, None);
        let mut structure = SchoolBBuffer::from_sparse(&uniform_with_color);
        structure.assemble_leaf_color(&uniform_with_color, |_| [1, 2, 3, 255]);
        assert!(
            uniform_with_color.has_uniform_materials(),
            "colour ≠ material id"
        );
        assert!(structure.has_leaf_color());
        let occupied = uniform_with_color.occupied_voxels();
        assert_eq!(occupied, 2 * LEAF_VOXELS as u64, "both bricks are full");
        let (est, dominant) = estimate_scene_bytes(&uniform_with_color, &structure);
        assert_eq!(
            est,
            2 * BASE_BYTES_PER_LEAF + occupied * TRUECOLOR_BYTES_PER_VOXEL
        );
        assert!(
            dominant,
            "truecolor (2*512*8=8192B) dwarfs zero dense-materials cost"
        );

        let dense_with_color = synthetic_tree(2, true, Some(9));
        let mut structure = SchoolBBuffer::from_sparse(&dense_with_color);
        structure.assemble_leaf_color(&dense_with_color, |_| [4, 5, 6, 255]);
        let occupied = dense_with_color.occupied_voxels();
        let (est, dominant) = estimate_scene_bytes(&dense_with_color, &structure);
        assert_eq!(
            est,
            2 * (BASE_BYTES_PER_LEAF + DENSE_MATERIALS_BYTES_PER_LEAF)
                + occupied * TRUECOLOR_BYTES_PER_VOXEL,
        );
        assert!(
            dominant,
            "truecolor (8192B) still exceeds dense materials (4096B)"
        );
    }

    #[test]
    fn budget_gate_passes_at_and_fails_past_a_small_explicit_budget() {
        // A tiny tree can never naturally reach WEB_SCENE_BUDGET_BYTES; test
        // the gate's boundary logic against an explicit small budget instead
        // of constructing a production-scale (hundreds-of-thousands-of-leaf)
        // fixture just to trip the real constant.
        let tree = synthetic_tree(4, false, None); // uniform: est = 4 * BASE_BYTES_PER_LEAF
        let structure = SchoolBBuffer::from_sparse(&tree);
        let est = 4 * BASE_BYTES_PER_LEAF;
        assert!(
            check_scene_budget_against(&tree, &structure, est).is_ok(),
            "exactly at budget"
        );
        let err = check_scene_budget_against(&tree, &structure, est - 1).expect_err("over budget");
        assert_eq!(err.leaf_count, 4);
        assert_eq!(err.est_bytes, est);
        assert!(!err.truecolor_dominant);
        let msg = err.to_string();
        assert!(msg.contains("scene too large"), "actionable message: {msg}");
        assert!(
            msg.contains("lower the resolution"),
            "actionable message: {msg}"
        );
        assert!(!msg.contains("truecolor"), "not the driver here: {msg}");
    }

    #[test]
    fn budget_error_hints_truecolor_only_when_it_dominates() {
        let dominant = SceneBudgetError {
            leaf_count: 10,
            est_bytes: 900_000_000,
            budget_bytes: 800_000_000,
            truecolor_dominant: true,
        };
        assert!(dominant.to_string().contains("disable truecolor"));

        let not_dominant = SceneBudgetError {
            leaf_count: 10,
            est_bytes: 900_000_000,
            budget_bytes: 800_000_000,
            truecolor_dominant: false,
        };
        assert!(!not_dominant.to_string().contains("truecolor"));
    }

    #[test]
    fn production_budget_is_generous_for_ordinary_scenes() {
        // A basic sanity check that the real constant (not the test-only
        // explicit-budget path) doesn't trip on a small everyday scene.
        let tree = synthetic_tree(100, false, None);
        let structure = SchoolBBuffer::from_sparse(&tree);
        assert!(check_scene_budget(&tree, &structure).is_ok());
    }
}
