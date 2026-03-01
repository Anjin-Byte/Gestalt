# CPU Ingestion Contract

Date: February 22, 2026
Status: Authoritative

Sources: `archive/voxelizer-chunk-native-output-design-requirements.md` §CPU Ingestion;
`archive/voxelizer-greedy-integration-spec.md` §§6, 10

---

## What This Document Defines

What the CPU does with the GPU compact output. The contract between
`design/gpu-output-contract.md` and the chunk manager's write API.

---

## Overview

The CPU receives `n_occupied` entries of `CompactVoxel {vx, vy, vz, material}`.
It does three things, in order:

1. **Group by chunk coordinate**
2. **Write each group into the chunk manager**
3. **Mark touched chunks dirty (deferred)**

No step is optional. The order matters for correctness (dirty marking deferred
until step 3 prevents greedy merge on partial chunk state).

---

## Step 1 — Group by Chunk Coordinate

```
chunks: HashMap<ChunkCoord, Vec<([u32;3], MaterialId)>>

for each CompactVoxel v in gpu_output:
    mat = if v.material == 0xFFFFFFFF || v.material == 0:
              MATERIAL_DEFAULT   // 1
          else:
              v.material as u16

    coord = ChunkCoord {
        x: v.vx.div_euclid(CS),   // CS = 62
        y: v.vy.div_euclid(CS),
        z: v.vz.div_euclid(CS),
    }
    local = [
        v.vx.rem_euclid(CS) as u32,
        v.vy.rem_euclid(CS) as u32,
        v.vz.rem_euclid(CS) as u32,
    ]
    chunks[coord].push((local, mat))
```

**Why `div_euclid` / `rem_euclid`:**
Euclidean division handles negative global voxel coordinates correctly. For
`vx = -1` and `CS = 62`:
- `div_euclid(-1, 62) = -1` (chunk at x=-1)
- `rem_euclid(-1, 62) = 61` (local slot 61 within that chunk)

Standard integer division would give wrong results for negative coordinates.

Source: `crates/greedy_mesher/src/chunk/coord.rs:82–116`.

---

## Step 2 — Write Each Chunk

For each `(coord, entries)` pair in the grouped map:

```
chunk = manager.get_or_create_chunk(coord)

for (local, mat) in entries:
    chunk.set_voxel_raw(local[0], local[1], local[2], mat)

chunk.increment_version()   // once per chunk, not per voxel
```

**`set_voxel_raw` behavior:**
Writes `mat` into the chunk's palette-based storage at local position `(x, y, z)`.
Internally applies the +1 padding offset: `self.voxels.set(x+1, y+1, z+1, mat)`.
The caller uses coordinates in `[0, 62)` — the padding offset is not the caller's
concern.

Source: `crates/greedy_mesher/src/chunk/chunk.rs:149–153`.

**`increment_version()` once per chunk:**
Version semantics track edit events, not voxel counts. A chunk receiving 1000
voxel writes from one voxelization call gets one version increment, not 1000.
The rebuild scheduler reacts to dirty marking (step 3), not to version changes
during a session.

---

## Step 3 — Dirty Marking (Deferred)

Dirty marking runs **after all chunks are written**. It does not interleave with
step 2.

```
touched_coords: HashSet<ChunkCoord>   // accumulated during step 2

for coord in touched_coords:
    manager.mark_dirty(coord)
    for neighbor in coord.face_neighbors():
        if manager.has_chunk(neighbor):
            manager.mark_dirty(neighbor)
```

**Why deferred:**
The greedy mesher rebuilds chunks by reading voxels from their neighbors for
correct boundary behavior. If dirty marking fires during step 2 (while some chunks
are only partially written), the rebuild scheduler may run the greedy merge before
all voxels of a session are in place. The resulting mesh is incorrect.

Deferring all dirty marking until after the final write ensures the rebuild
scheduler always sees a complete, consistent chunk state.

**Why neighbors are marked:**
A voxel on the face of chunk A is part of the rendering boundary with chunk B.
Greedy quads that span the boundary are reconstructed only when both A and B are
rebuilt. Writing voxels into A without marking B dirty would leave the rendering
boundary stale.

---

## Partial Fill Policy (VOX-PARTIAL)

The voxelizer grid covers `dims[0] × dims[1] × dims[2]` voxels. This grid
generally does not align to chunk boundaries. A chunk partially overlapping the
grid will receive writes only for the voxels inside the grid; the voxels outside
the grid are not touched.

**Policy:** the voxelizer writes only the voxels it has data for. It does not
clear, zero, or otherwise modify voxels in partially-covered chunks that fall
outside the grid.

**Rationale:** the chunk manager is a persistent mutable world store. Other edits
(player edits, other mesh placements) may exist in the same chunk. A voxelization
session places a mesh into the world; it does not own or clear the surrounding
space. If the application needs to clear a region before voxelizing (to replace
an object), it must do so explicitly via the chunk manager's clear API before
issuing the voxelization call.

---

## Relationship to `set_voxels_batch`

The existing `ChunkManager::set_voxels_batch`
(`crates/greedy_mesher/src/chunk/manager.rs:214–248`) performs equivalent
semantics with a different calling convention:

| Aspect | `set_voxels_batch` | CPU ingestion (this design) |
|--------|--------------------|-----------------------------|
| Input | `&[([f32;3], MaterialId)]` — world floats | `&[CompactVoxel]` — global i32 + material u32 |
| Coord conversion | Converts world float to chunk coord internally | GPU already provides global i32; CPU does `div_euclid` |
| Grouping | HashMap inside the call | HashMap in the ingestion layer before calling chunk manager |
| Per-chunk write | `set_voxel_raw` per voxel | Same |
| Version increment | Once per chunk per call | Same |
| Dirty marking | Immediately, inside the call | Deferred — returned coord set for later |

The new ingestion layer replaces `set_voxels_batch` as the write path for
voxelizer output. The `set_voxels_batch` API remains unchanged and continues to
serve other data sources (player edits, procedural fills, etc.).
