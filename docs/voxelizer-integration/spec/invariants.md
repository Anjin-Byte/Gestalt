# Formal Invariants and Correctness

**Type:** spec
**Status:** current
**Date:** 2026-02-22

Sources: `archive/voxelizer-greedy-integration-spec.md` §§9.5, 10.4;
`archive/voxelizer-chunk-native-output-design-requirements.md` §Constraints

---

## Scope

All invariants that must hold at every call boundary and throughout execution.
An implementer checking their work against these invariants should be able to
verify correctness without consulting any other document.

---

## Pre-call Invariants (validated before GPU work begins)

### VOX-ALIGN

**Formal:** For each `i ∈ {x, y, z}`:

```
(origin_world[i] / voxel_size - round(origin_world[i] / voxel_size)).abs() < 1e-4
```

**Checked by:** `voxelize_and_apply` at call entry, before GPU dispatch.

**Consequence of violation:** Voxels near chunk boundaries are written into the
wrong chunk, producing seam artifacts in the greedy mesh.

**Enforcement:** Hard rejection. The Promise rejects with an explanatory error.
The caller is responsible for snapping origin to a voxel-aligned value.

See `spec/coordinate-frames.md` for full derivation and enforcement code.

### VOX-SIZE

**Formal:** `voxel_size` used for voxelization must equal `manager.voxel_size()`.

**Checked by:** `voxelize_and_apply` reads `voxel_size` from the chunk manager —
the caller does not supply it as a parameter. Structural enforcement: it is
impossible to pass the wrong value.

**Consequence of violation:** Voxels appear at offset world positions relative to
the chunk manager's coordinate system.

### Material table length

**Formal:** `material_table.len() == indices.len() / 3`

**Checked by:** `voxelize_and_apply` before copying typed arrays.

**Consequence of violation:** Triangle indices in the GPU compact pass would read
out of bounds of the material table. The shader has a bounds guard (`raw_owner <
arrayLength * 2u`) that falls through to `MATERIAL_DEFAULT`, but the underlying
cause is a caller error that should be caught early.

---

## Correctness Invariants (during and after execution)

### C1 — Occupancy Conservation

**Formal:** The number of entries in the compact output equals the number of set
bits in the voxelizer's occupancy arrays:

```
compact_output.len() == sum over all bricks of popcount(occupancy[brick])
```

**Checked by:** Debug assertion in the compact pass readback (if enabled).

**Consequence of violation:** Occupied voxels were lost during compaction. The
resulting mesh is missing voxels that should appear.

### C2 — Local Coordinate Range

**Formal:** For all `(lx_c, ly_c, lz_c)` produced by the CPU ingestion:

```
lx_c in [0, 62)  ∧  ly_c in [0, 62)  ∧  lz_c in [0, 62)
```

**Follows from:** `rem_euclid(vx, CS=62)` always returns a value in `[0, 62)`
regardless of the sign of `vx`. This is a mathematical property of Euclidean
remainder, not a conditional check.

**Verified by:** `crates/greedy_mesher/src/chunk/coord.rs:201–236` (existing tests
for the `rem_euclid` property).

**Consequence of violation:** `set_voxel_raw` would reject the write (it guards
`x >= Self::SIZE = 62`) and the voxel would be silently lost.

### C3 — Material Validity

**Formal:** No entry written via `set_voxel_raw` has `material == MATERIAL_EMPTY (0)`.

**Enforced by:** CPU sentinel check before every write:

```rust
let mat = if v.material == 0xFFFF_FFFF || v.material == 0 {
    MATERIAL_DEFAULT  // 1
} else {
    v.material as MaterialId
};
```

**Consequence of violation:** A voxel with `MATERIAL_EMPTY = 0` in chunk storage
is treated as air by the greedy mesher. The voxel appears occupied in the
occupancy mask but generates no visible face — a phantom voxel.

### C4 — Chunk Coordinate Round-Trip

**Formal:** For all written `(cx, cy, cz, lx_c, ly_c, lz_c)`:

```
div_euclid(cx * CS + lx_c, CS) == cx
rem_euclid(cx * CS + lx_c, CS) == lx_c
```

**Follows from:** Standard property of Euclidean division. Tested in
`crates/greedy_mesher/src/chunk/coord.rs:201–236`.

**Consequence of violation:** The chunk manager would reject the write or store
the voxel in the wrong local slot.

---

## Operational Invariants (sequencing and lifecycle)

### Deferred Dirty Marking

**Formal:** `mark_dirty(coord)` is called only after all `set_voxel_raw` calls for
all chunks in a session are complete.

**Enforced by:** `compact_to_chunk_writes` accumulates touched `ChunkCoord` values
into a `HashSet` and calls `mark_dirty` only at the end of the function, after
the write loop exits.

**Consequence of violation:** The rebuild scheduler may rebuild a chunk while other
chunks in the same session are still being written. The greedy merge would see
partial chunk state and produce incorrect boundary faces.

### Version Increment Once Per Chunk

**Formal:** Each chunk touched by a call to `compact_to_chunk_writes` receives
exactly one `increment_version()` call, regardless of how many voxels were written
into it.

**Enforced by:** The write loop calls `increment_version()` once per chunk in the
grouped map, not once per voxel.

**Rationale:** Version semantics track edit events, not voxel counts. The rebuild
scheduler reacts to dirty marking, not to version changes. Per-voxel version
increments would be harmless but wasteful.

### No Empty Chunks Created

**Formal:** `get_or_create_chunk(coord)` is called only for chunks that receive
at least one `set_voxel_raw` write.

**Follows from:** The grouping step only creates `HashMap` entries for chunks
containing at least one voxel from the compact output.

**Consequence of violation:** Empty chunks allocated unnecessarily consume memory
and trigger unnecessary mesh rebuilds when marked dirty.

### Partial Fill Does Not Clear

**Formal:** The CPU ingestion layer does not zero, clear, or modify any voxel in
a partially-covered chunk that falls outside the voxelizer grid.

**Follows from:** The CPU ingestion layer writes only what the compact output
contains. It does not iterate chunk voxels; it only processes `CompactVoxel`
entries.

**Consequence of violation:** Would silently destroy existing world state in chunks
that happen to share space with a newly voxelized mesh.

---

## Debug Assertions Reference

In debug builds, the following assertions may be enabled:

```rust
// C1 — Occupancy conservation (verify compact pass)
debug_assert_eq!(
    compact_voxels.len(),
    expected_occupied_count,
    "compact pass lost voxels"
);

// C2 — Local coordinate range
for &local in &[lx_c, ly_c, lz_c] {
    debug_assert!(local < 62, "local coord out of range");
}

// C3 — Material validity
debug_assert_ne!(mat, MATERIAL_EMPTY, "MATERIAL_EMPTY written to solid voxel");
```
