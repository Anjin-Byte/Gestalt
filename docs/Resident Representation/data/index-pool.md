# Index Pool

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Derived data (R-1) — rebuilt from chunk occupancy by the greedy mesher.

> Shared GPU index buffer for all chunk meshes. Triangles are defined by u32 indices into the vertex pool.

---

## Identity

- **Buffer name:** `index_pool`
- **WGSL type:** `array<u32>`
- **GPU usage:** `INDEX | STORAGE | COPY_DST`
- **Binding:** used as index buffer in R-2 (depth prepass), R-5 (main color pass); storage read in meshlet build pass

---

## Layout

Each index is a **u32** (4 bytes), referencing a vertex within the parent chunk's region of the `vertex_pool`.

Indices for all resident chunks are packed into a single flat buffer. Per-chunk regions are located via `draw_metadata[slot]`, which records the index offset and count.

```
For chunk at slot S:
  meta = draw_metadata[S]
  first_index_byte = meta.index_offset * 4
  index_count      = meta.index_count

  triangle T (0-based) has indices:
    vertex_pool[meta.vertex_offset + index_pool[meta.index_offset + T*3 + 0]]
    vertex_pool[meta.vertex_offset + index_pool[meta.index_offset + T*3 + 1]]
    vertex_pool[meta.vertex_offset + index_pool[meta.index_offset + T*3 + 2]]
```

Indices are **chunk-local** — they are relative to the chunk's `vertex_offset` in the vertex pool. The `baseVertex` field in `DrawIndexedIndirect` args applies the offset at draw time.

### Greedy Mesh Index Pattern

Each greedy-meshed quad emits 4 vertices and 6 indices (two triangles). Index patterns are always:

```
quad Q (0-based within chunk):
  indices: [Q*4+0, Q*4+1, Q*4+2, Q*4+2, Q*4+1, Q*4+3]
```

This is a fixed winding order for consistent front-face determination with backface culling.

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| IP-1 | An index at position `draw_metadata[slot].index_offset + i` belongs exclusively to chunk `slot` | Mesh rebuild pass writes non-overlapping regions |
| IP-2 | Every index value is in range `[0, draw_metadata[slot].vertex_count)` | Greedy mesher output validation |
| IP-3 | `index_count` is always a multiple of 3 (complete triangles only) | Greedy mesher postcondition |
| IP-4 | Index data for a slot is valid only when `mesh_version[slot] == chunk_version[slot]` | Version check before read |
| IP-5 | A slot with `chunk_resident_flags[slot] == 0` has undefined index pool content | Pool lifecycle |
| IP-6 | Total pool size = `MAX_SLOTS * MAX_INDICES_PER_CHUNK * 4` bytes (Option A fixed reservation) | Buffer creation |

---

## Valid Value Ranges

| Field | Range | Notes |
|---|---|---|
| Each u32 index | `0 .. draw_metadata[slot].vertex_count - 1` | Chunk-local; baseVertex applied at draw time |
| `index_offset` (in draw_metadata) | `0 .. total_pool_capacity - 1` | Out-of-range = buffer overrun |
| `index_count` (in draw_metadata) | `0 .. MAX_INDICES_PER_CHUNK` | Always divisible by 3; 0 for empty chunks |

---

## Producers

| Producer | When | What it writes |
|---|---|---|
| Greedy mesher (R-1) | Mesh rebuild for dirty chunks | Full index region for rebuilt chunk |

Currently runs on CPU (Rust, Web Worker). Future target: GPU compute writing directly into `index_pool`.

---

## Consumers

| Consumer | Stage | How it reads |
|---|---|---|
| Depth prepass | R-2 | Index fetch via `draw_metadata` offsets |
| Meshlet build pass | Post R-1 | Reads chunk index data to partition into per-meshlet index ranges in `meshlet_index_pool` |
| Main color pass | R-5 | Index fetch via `indirect_draw_buf` args (firstIndex, indexCount) |
| Debug viz | R-9 | Wireframe re-render via same draw path |

---

## Underspecified

> **DESIGN GAP:** The narrative documents ([gpu-chunk-pool](../gpu-chunk-pool.md), [pipeline-stages](../pipeline-stages.md)) do not state concrete values for `MAX_INDICES_PER_CHUNK` or total pool capacity. Since each greedy-meshed quad produces 6 indices, and `MAX_INDICES_PER_CHUNK = MAX_QUADS_PER_CHUNK * 6`, the index budget depends on the worst-case quad count for a 64-cubed chunk. These must be determined by profiling and stated as named constants.
>
> Candidate values (to be validated):
> - `MAX_INDICES_PER_CHUNK`: 393,216 (64K quads * 6) — theoretical worst case
> - Typical dense chunk: far fewer quads due to greedy merge
> - Total pool capacity: `MAX_SLOTS * MAX_INDICES_PER_CHUNK` indices
> - At 1024 slots and 393K indices: 1024 * 393216 * 4B = ~1.5 GB (Option A worst case; typical usage far lower)
>
> The Option A worst-case memory is prohibitive. This reinforces the narrative docs' note that Option B (variable allocation with freelist) will be needed. Realistic per-chunk budgets based on profiled greedy mesh output should be established before committing to allocation sizes.

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Triangle completeness:** Every emitted index_count is divisible by 3.
2. **Index bounds:** Every index value is less than the corresponding vertex_count for its chunk.
3. **Region isolation:** After mesh rebuild for slot S, verify no bytes outside `[index_offset, index_offset + index_count)` were modified.
4. **Winding consistency:** For each quad's two triangles, verify front-face winding matches expected normal direction.

### Property tests (Rust, randomized)

5. **Offset non-overlap:** For any two resident slots S1, S2, their `[index_offset, index_offset + index_count)` ranges do not overlap.
6. **Capacity bound:** For randomized chunk occupancy, greedy mesher index count never exceeds `MAX_INDICES_PER_CHUNK`.
7. **Quad ratio:** Verify `index_count == (vertex_count / 4) * 6` for greedy mesh output (4 verts per quad, 6 indices per quad).

### GPU validation (WGSL compute)

8. **Readback test:** After mesh rebuild, dispatch compute shader that reads indices via `draw_metadata`, verify all indices dereference to valid vertex positions within chunk world-space bounds.
9. **Degenerate triangle check:** Verify no triangle has two identical indices (degenerate) — greedy mesher should never emit these.

---

## See Also

- [vertex-pool](vertex-pool.md) — companion vertex buffer, same pool pattern
- [chunk-occupancy-atlas](chunk-occupancy-atlas.md) — authoritative source that mesh is derived from
- [gpu-chunk-pool](../gpu-chunk-pool.md) — mesh pool design, Option A vs Option B allocation
- [pipeline-stages](../pipeline-stages.md) — R-1 (mesh rebuild), R-2 (depth prepass), R-5 (color pass)
- [indirect-draw-buf](indirect-draw-buf.md) — consumes index offsets and counts at draw time
