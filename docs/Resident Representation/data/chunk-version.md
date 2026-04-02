# Chunk Version

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Authoritative data — never derived, written by producers only.

> Monotonic version counter per chunk slot, used to detect staleness of all derived products (mesh, summary, GI cache, meshlets).

---

## Identity

- **Buffer name:** `chunk_version` (GPU), plus CPU-side `data_version: u64` per-chunk metadata
- **WGSL type:** `array<u32>` indexed by slot (GPU); `u64` per-chunk metadata (CPU)
- **GPU usage:** `STORAGE` (read/write by edit kernels via `atomicAdd`)
- **Binding:** `@group(0) @binding(?)` in edit, summary, and mesh shaders

---

## Layout

One slot occupies **4 bytes** (1 × `u32`).

```
chunk_version[slot]   — u32, GPU-side, incremented atomically by edit kernels
data_version          — u64, CPU-side per-chunk metadata, monotonic counter
```

GPU-side access:

```
let ver = atomicLoad(&chunk_version[slot]);
```

After an edit kernel writes occupancy or material data:

```
atomicAdd(&chunk_version[slot], 1u);
```

### Related per-slot version tags

Each derived product records the `chunk_version` value at the time it was built:

| Tag | What it tracks |
|---|---|
| `mesh_version[slot]` | Version when mesh (vertex/index pool) was last rebuilt |
| `summary_version[slot]` | Version when occupancy summary was last rebuilt |
| `gi_cache_version[slot]` | Version when GI cache (radiance cascade data) was last rebuilt |
| `meshlet_version[slot]` | Version when meshlet subdivision was last rebuilt |

A derived product is **current** when its version tag equals `chunk_version[slot]`. It is **stale** when the tag is less than `chunk_version[slot]`.

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| VER-1 | `chunk_version[slot]` is monotonically non-decreasing within a slot allocation lifetime | Edit kernels use `atomicAdd` only |
| VER-2 | `chunk_version[slot]` is reset to 0 on slot eviction/recreation | CPU pool manager (reset on eviction) |
| VER-3 | Edit kernels must `atomicAdd(&chunk_version[slot], 1u)` AFTER writing occupancy/material data, never before | Edit kernel postcondition ordering |
| VER-4 | `mesh_version[slot]` <= `chunk_version[slot]` at all times | Mesh rebuild writes `mesh_version = chunk_version` only after completion |
| VER-5 | If `mesh_version[slot] == chunk_version[slot]`, the mesh artifact is current | Staleness check definition |
| VER-6 | If `mesh_version[slot] != chunk_version[slot]`, the mesh is stale and must be rebuilt before use | Staleness check definition |

---

## Valid Value Ranges

| Field | Range | Notes |
|---|---|---|
| `chunk_version[slot]` (GPU) | `0 .. 0xFFFFFFFF` | Wraps at u32 max; practical overflow is unlikely within a session |
| `data_version` (CPU) | `0 .. u64::MAX` | Monotonic; never wraps in practice |
| `slot_index` | `0 .. MAX_SLOTS-1` | Out-of-range = buffer overrun |
| Derived version tags | `0 .. chunk_version[slot]` | Must never exceed the chunk version |

---

## Producers

| Producer | When | What it writes |
|---|---|---|
| Edit kernels | Runtime voxel edits | `atomicAdd(&chunk_version[slot], 1u)` after every occupancy/material write |
| CPU pool manager | Slot eviction/recreation | Reset `chunk_version[slot]` to 0 via buffer clear or `writeBuffer` |

---

## Consumers

| Consumer | Stage | How it reads |
|---|---|---|
| Mesh rebuild | R-1 | Compares `mesh_version[slot]` against `chunk_version[slot]` to detect staleness |
| Summary rebuild | I-3 | Compares `summary_version[slot]` against `chunk_version[slot]` to detect staleness |
| Swap validation | Pool lifecycle | Reads `chunk_version[slot]` to verify consistency before slot reuse |
| GI cache rebuild | R-6 | Compares `gi_cache_version[slot]` against `chunk_version[slot]` |

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Reset on eviction:** Allocate slot, increment version N times, evict, verify version reads 0.
2. **Monotonicity:** Perform a sequence of edits, read version after each — values are strictly increasing.
3. **CPU data_version isolation:** Incrementing `data_version` for chunk A does not affect chunk B.

### Property tests (Rust, randomized)

4. **Staleness detection:** For random edit counts, verify `mesh_version != chunk_version` iff there has been an edit since last mesh rebuild.
5. **Slot isolation:** Incrementing version on slot N does not affect slot N+1 or N-1.

### GPU validation (WGSL compute)

6. **Atomic ordering:** Dispatch edit kernel that writes occupancy then increments version; readback confirms version >= 1 and occupancy is set.
7. **Concurrent edit test:** Dispatch multiple workgroups editing different voxels in the same slot; readback confirms `chunk_version[slot]` equals the number of edit invocations.
