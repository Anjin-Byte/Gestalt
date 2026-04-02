# Chunk AABB

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Derived (produced by I-3 summary rebuild pass).

> World-space tight axis-aligned bounding box of occupied voxels within a chunk. Primary input to frustum cull, Hi-Z occlusion test, and ray-AABB intersection for DDA entry.

---

## Identity

- **Buffer name:** `chunk_aabb`
- **WGSL type:** `array<vec4f>` (2 vec4f per slot: min + max)
- **GPU usage:** `STORAGE`
- **Binding:** read by R-2 (depth prepass draw), R-4 (frustum cull, Hi-Z occlusion test), R-6 (ray-AABB intersection for DDA entry)

---

## Layout

One slot occupies **32 bytes** (2 x `vec4f` = 2 x 16 bytes).

```
For slot S:
  chunk_aabb[S * 2 + 0] = vec4f(world_min.x, world_min.y, world_min.z, 0.0)
  chunk_aabb[S * 2 + 1] = vec4f(world_max.x, world_max.y, world_max.z, 0.0)
```

The `.w` components of both vec4f are reserved and must be 0.0.

### World-Space Computation

Computed by I-3 from `chunk_occupancy_atlas` and `chunk_coord`:

```
// local_min, local_max: voxel-space tight bounds of occupied voxels
//   found by scanning opaque_mask for min/max occupied (x, y, z)

world_min = chunk_coord * 62 * voxel_size + local_min * voxel_size
world_max = chunk_coord * 62 * voxel_size + (local_max + vec3(1, 1, 1)) * voxel_size
```

The `+1` on `local_max` accounts for voxel extent: a voxel at position `(x, y, z)` occupies the volume `[x, x+1) x [y, y+1) x [z, z+1)` in local space.

The factor `62` is the chunk stride (64 minus the 1-voxel padding on each side).

### Empty Chunk Convention

For empty chunks (no occupied voxels), the AABB is degenerate:

```
world_min = vec4f(+INF, +INF, +INF, 0.0)
world_max = vec4f(-INF, -INF, -INF, 0.0)
```

This ensures any frustum or ray-AABB test against a degenerate AABB returns false (no intersection), so empty chunks are naturally excluded from all spatial queries without special-case logic.

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| AABB-1 | For non-empty chunks, `min < max` component-wise (x, y, z) | Summary rebuild pass (I-3) |
| AABB-2 | Every occupied voxel in `chunk_occupancy_atlas` is inside the AABB | Summary rebuild pass scans all occupied voxels |
| AABB-3 | At least one occupied voxel touches each face of the AABB (tightness) | Summary rebuild computes exact min/max of occupied extent |
| AABB-4 | Empty chunks have degenerate AABB (`min > max` or `min == max`) | Summary rebuild sets degenerate values when occupancy is all-zero |
| AABB-5 | The `.w` components of both vec4f are 0.0 (reserved) | Summary rebuild pass writes 0.0 to .w |
| AABB-6 | `summary_version[slot]` matches `chunk_version[slot]` when the AABB is fresh | Summary rebuild postcondition |
| AABB-7 | A slot with `chunk_resident_flags[slot] == 0` has undefined AABB content | Pool lifecycle |
| AABB-8 | Total buffer size = `MAX_SLOTS * 2 * 16` bytes | Buffer creation |

---

## Valid Value Ranges

| Field | Range | Notes |
|---|---|---|
| `world_min.xyz` | Any finite f32 | World-space position; can be negative for negative chunk coords |
| `world_max.xyz` | Any finite f32 | Must be > world_min component-wise for non-empty chunks |
| `.w` components | `0.0` | Reserved |
| `slot_index` | `0 .. MAX_SLOTS-1` | Out-of-range = buffer overrun |

---

## Producers

| Producer | When | What it writes |
|---|---|---|
| Summary rebuild pass (I-3) | After occupancy upload or edit | Full 32 bytes per slot (2 x vec4f), computed from `chunk_occupancy_atlas` + `chunk_coord` |

---

## Consumers

| Consumer | Stage | How it reads |
|---|---|---|
| Frustum cull | R-4 phase 1 | AABB-vs-frustum test; chunks outside frustum are culled |
| Hi-Z occlusion test | R-4 phase 1 | AABB projected to screen-space; tested against Hi-Z pyramid |
| Ray-AABB intersection | R-6 | Compute ray entry/exit t-values for DDA entry into chunk |
| Depth prepass draw | R-2 | Chunk AABB used for front-to-back sort ordering |

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Addressing correctness:** For a known slot index, verify the min/max vec4f are at the correct buffer offset.
2. **Tight bounds:** Place occupied voxels at known positions, verify AABB min/max match exactly.
3. **Single voxel:** One occupied voxel at (x, y, z) produces AABB from `(x, y, z)` to `(x+1, y+1, z+1)` in local space, correctly offset to world space.
4. **Empty chunk:** All-zero occupancy produces degenerate AABB where min > max.
5. **Full chunk:** All-occupied produces AABB covering the entire 64x64x64 volume.

### Property tests (Rust, randomized)

6. **Containment:** Generate random occupancy, compute AABB, verify every occupied voxel is inside the AABB.
7. **Tightness:** For each face of the AABB, verify at least one occupied voxel touches it.
8. **Slot isolation:** Computing AABB for slot N does not affect slot N+1 or N-1.
9. **World-space offset:** For different chunk_coord values, verify world_min and world_max are correctly offset.

### GPU validation (WGSL compute)

10. **Readback test:** Write known occupancy pattern from CPU, dispatch summary rebuild, readback AABB, verify against CPU reference.
