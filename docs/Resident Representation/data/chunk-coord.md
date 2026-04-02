# Chunk Coord

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Authoritative data — never derived, written by producers only.

> Chunk-space integer coordinates for every resident slot. The single source of truth for where a chunk lives in the world.

---

## Identity

- **Buffer name:** `chunk_coord`
- **WGSL type:** `array<vec4i>` (one `vec4i` per slot)
- **GPU usage:** `STORAGE | COPY_DST`
- **Binding:** `@group(0) @binding(TBD)` in cull, traversal, and reconstruction shaders
- **Size:** 16 bytes per slot (`i32` x 3 + 4 bytes padding)

---

## Layout

One slot occupies **16 bytes** (one `vec4i`).

```
chunk_coord[slot] = vec4i(x, y, z, 0)

Where:
  x, y, z  = chunk-space integer coordinates (signed i32)
  w        = reserved, must be 0

World-space origin reconstruction:
  world_origin = chunk_coord[slot].xyz * 62 * voxel_size
```

The factor 62 (not 64) accounts for the 1-voxel padding ring on each side of the 64x64x64 chunk. Only the interior 62x62x62 voxels represent unique spatial content; the padding duplicates neighbor boundaries.

### Why vec4i?

GPU uniform/storage alignment requires 16-byte alignment for vec types. A `vec3i` would be padded to 16 bytes anyway. Using `vec4i` makes the padding explicit and reserves `.w` for future use (e.g., LOD level, generation counter).

### Coordinate space

Coordinates are signed integers, supporting negative chunk positions. The chunk at coordinate (0, 0, 0) has its world-space origin at the world origin. Chunks at (-1, 0, 0) are to the negative-X side, and so on.

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| COORD-1 | `chunk_coord[slot].w == 0` | CPU pool manager write; GPU validation shader |
| COORD-2 | No two resident slots have the same (x, y, z) coordinate | CPU pool manager dedup check on allocation |
| COORD-3 | `chunk_coord` is set before any derived data is computed for this slot | CPU pool manager allocation ordering |
| COORD-4 | `chunk_coord` is immutable for the lifetime of a slot allocation (until eviction) | CPU pool manager — no writes after initial set until slot is freed |

---

## Valid Value Ranges

| Field | Range | Notes |
|---|---|---|
| `x`, `y`, `z` | `i32::MIN .. i32::MAX` | Practical range limited by world bounds; full i32 range is valid |
| `w` | `0` | Reserved — must be 0 per COORD-1 |
| `slot` | `0 .. MAX_SLOTS-1` | Out-of-range = buffer overrun |

---

## Producers

| Producer | When | What it writes |
|---|---|---|
| CPU pool manager | On chunk load / slot allocation | Full `vec4i` for newly resident chunk |

---

## Consumers

| Consumer | Stage | How it reads |
|---|---|---|
| AABB world-space computation | I-3 | `coord.xyz * 62 * voxel_size` to derive chunk AABB min/max |
| Frustum / occlusion cull | R-4 | Reads coord for world-space AABB test against view frustum |
| World position reconstruction | R-5 | Reconstructs voxel world position from chunk coord + local voxel offset |
| DDA traversal coord-to-slot lookup | R-6 | Maps world-space ray position to chunk coord, then looks up slot via `chunk_slot_table_gpu` |

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Write/read roundtrip:** Write a `vec4i` to a slot, read back — xyz match, w == 0.
2. **Negative coordinates:** Write coords with negative x, y, z values — read back correctly.
3. **World-space reconstruction:** For known coords and voxel_size, verify `coord.xyz * 62 * voxel_size` produces the expected world origin.
4. **Uniqueness enforcement:** Attempt to allocate two slots with the same (x, y, z) — second allocation is rejected.
5. **Immutability:** After allocation, verify no writes succeed until the slot is freed.

### Property tests (Rust, randomized)

6. **Roundtrip with random coords:** Generate random `vec4i` values (w forced to 0), write to random slots, read back — all match.
7. **Slot isolation:** Writing coord to slot N does not affect slot N+1 or N-1.
8. **No-duplicate property:** Allocate N random unique coords — all succeed. Re-allocate any existing coord — fails.

### GPU validation (WGSL compute)

9. **Readback test:** Write known coords from CPU, dispatch compute shader that reads every slot's coord, verify via readback buffer.
10. **W-channel validation:** Dispatch shader that checks `chunk_coord[slot].w == 0` for all resident slots, report violations.
