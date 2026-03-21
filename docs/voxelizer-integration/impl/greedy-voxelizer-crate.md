# `crates/greedy_voxelizer` — CPU Ingestion Crate

**Type:** spec
**Status:** proposed
**Date:** 2026-02-22

---

## Purpose

`crates/greedy_voxelizer` is a pure Rust library (no WASM, no JS). It bridges
the GPU compact output (`Vec<CompactVoxel>`) and the chunk manager's write API.
It has no dependency on the WASM environment and can be tested with a wgpu Vulkan
backend in native builds.

This crate replaces the old `sparse_to_chunk_edits` approach. Key difference:
the GPU has already scanned occupancy bits and resolved materials. This crate does
only what the GPU cannot: group by chunk coordinate and write into the palette.

---

## File Structure

```
crates/greedy_voxelizer/
  Cargo.toml
  src/
    lib.rs      — pub use compact_to_chunk_writes
    ingest.rs   — core grouping and write logic
```

---

## `Cargo.toml`

```toml
[package]
name = "greedy_voxelizer"
version = "0.1.0"
edition = "2021"

[dependencies]
greedy_mesher = { path = "../greedy_mesher" }
voxelizer     = { path = "../voxelizer" }
```

No WASM-specific dependencies. `voxelizer` is needed only for the `CompactVoxel`
type from `crates/voxelizer/src/core.rs`.

---

## `src/lib.rs`

```rust
mod ingest;
pub use ingest::compact_to_chunk_writes;
```

---

## `src/ingest.rs`

### Function Signature

```rust
use std::collections::{HashMap, HashSet};
use greedy_mesher::{
    chunk::{manager::ChunkManager, coord::ChunkCoord},
    core::{MaterialId, MATERIAL_DEFAULT, MATERIAL_EMPTY, CS},
};
use voxelizer::core::CompactVoxel;

/// Convert GPU compact voxel output into ChunkManager writes.
///
/// Groups entries by chunk coordinate using Euclidean division.
/// Writes each voxel via `set_voxel_raw` and increments version once per chunk.
/// Marks touched chunks and their face neighbors dirty after all writes.
///
/// Preconditions:
///   - voxels come from the GPU compact pass with Architecture B output
///   - caller has validated VOX-ALIGN and VOX-SIZE before GPU dispatch
///
/// Returns the count of voxels written (== voxels.len() minus any filtered sentinels).
pub fn compact_to_chunk_writes(
    voxels:  &[CompactVoxel],
    manager: &mut ChunkManager,
) -> usize
```

### Full Algorithm

```rust
pub fn compact_to_chunk_writes(
    voxels:  &[CompactVoxel],
    manager: &mut ChunkManager,
) -> usize {
    let cs = CS as i32;   // 62

    // Step 1: Group by chunk coordinate
    let mut by_chunk: HashMap<ChunkCoord, Vec<([u32; 3], MaterialId)>> =
        HashMap::with_capacity(voxels.len() / 16);   // rough estimate

    for v in voxels {
        // Resolve material — handle sentinel and zero
        let mat: MaterialId = if v.material == 0xFFFF_FFFF || v.material == 0 {
            MATERIAL_DEFAULT
        } else {
            // material fits in u16 by GPU contract (was a u16 before packing)
            v.material as MaterialId
        };

        // Euclidean division: handles negative global voxel coords correctly
        let coord = ChunkCoord::new(
            v.vx.div_euclid(cs),
            v.vy.div_euclid(cs),
            v.vz.div_euclid(cs),
        );
        let local = [
            v.vx.rem_euclid(cs) as u32,   // in [0, 62) by construction
            v.vy.rem_euclid(cs) as u32,
            v.vz.rem_euclid(cs) as u32,
        ];

        by_chunk.entry(coord).or_default().push((local, mat));
    }

    // Step 2: Write each chunk
    let touched: Vec<ChunkCoord> = by_chunk.keys().copied().collect();
    let mut count = 0usize;

    for (coord, entries) in &by_chunk {
        let chunk = manager.get_or_create_chunk(*coord);
        for (local, &mat) in entries {
            // Invariant C3: mat != MATERIAL_EMPTY (guaranteed by sentinel check above)
            chunk.set_voxel_raw(local[0], local[1], local[2], mat);
            count += 1;
        }
        // One version increment per chunk, not per voxel
        chunk.increment_version();
    }

    // Step 3: Deferred dirty marking — after ALL chunks are written
    for &coord in &touched {
        manager.mark_dirty(coord);
        for neighbor in coord.face_neighbors() {
            if manager.has_chunk(neighbor) {
                manager.mark_dirty(neighbor);
            }
        }
    }

    count
}
```

### Notes on the Algorithm

**`HashMap::with_capacity` estimate:** For a typical OBJ file, `voxels.len() / 16`
is a conservative estimate of the number of unique chunks touched (each chunk holds
up to 62³ = ~238K voxels; 16 is a tunable heuristic). This avoids most rehashing
without over-allocating.

**`div_euclid` / `rem_euclid`:** Required for negative global coordinates. See
`spec/coordinate-frames.md` for the full explanation.

**Deferred dirty marking:** All `set_voxel_raw` calls complete before any
`mark_dirty` call. This prevents the rebuild scheduler from running a greedy
merge on partially-written chunk state. See `spec/invariants.md` (Deferred Dirty
Marking invariant).

**`face_neighbors()`:** Returns the six axis-aligned neighbors of a chunk
coordinate. Marking neighbors dirty ensures greedy quads that span the boundary
between chunk A and chunk B are correctly rebuilt when A is written.

---

## How This Differs from Old `sparse_to_chunk_edits`

The old `sparse_to_chunk_edits` (Architecture A) was responsible for:

1. Iterating all brick occupancy bits (O(total_grid_voxels) CPU scan)
2. Looking up `material_table[owner_id]` per voxel
3. Computing world-space coordinates from `brick_origin + local_xyz`
4. Grouping by chunk coordinate
5. Calling `set_voxels_batch`

`compact_to_chunk_writes` does only steps 4 and 5. Steps 1-3 are done by the GPU.
The function is shorter, simpler, and scales with `n_occupied` not `grid_volume`.

---

## Debug Assertions

Enable in debug builds to verify correctness invariants:

```rust
// C2 — Local coordinate range
debug_assert!(local[0] < 62 && local[1] < 62 && local[2] < 62,
    "local coord out of [0, 62) range: {:?}", local);

// C3 — Material validity
debug_assert_ne!(mat, MATERIAL_EMPTY,
    "MATERIAL_EMPTY written to occupied voxel");
```
