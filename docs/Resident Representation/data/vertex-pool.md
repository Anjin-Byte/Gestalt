# Vertex Pool

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Derived data (R-1) — rebuilt from chunk occupancy by the greedy mesher.

> Large shared GPU buffer holding all chunk mesh vertices. The geometry backbone for raster passes.

---

## Identity

- **Buffer name:** `vertex_pool`
- **WGSL type:** `array<f32>` (interpreted as interleaved `vec3f` position + `u32` packed normal+material per vertex)
- **GPU usage:** `STORAGE | COPY_DST`
- **Binding:** bound in R-2 (depth prepass), R-5 (main color pass)

---

## Layout

Each vertex occupies **16 bytes**:

```
struct PackedVertex {
    position: vec3f,          // 12 bytes — world-space vertex position
    normal_material: u32,     // 4 bytes  — packed normal (3 × snorm8) + material ID (u8)
}
```

Vertices for all resident chunks are packed contiguously into a single flat buffer. Per-chunk regions are located via `draw_metadata[slot]`, which records the vertex offset and count for each chunk's mesh.

```
For chunk at slot S:
  meta = draw_metadata[S]
  first_vertex_byte = meta.vertex_offset * 16
  vertex_count      = meta.vertex_count

  vertex[i] = vertex_pool[meta.vertex_offset + i]   // i in [0, vertex_count)
```

### Packing Convention

The `normal_material` u32 is laid out as:

```
bits [31:24]  material_id    u8    index into global material_table
bits [23:16]  normal_z       i8    snorm8
bits [15:8]   normal_y       i8    snorm8
bits [7:0]    normal_x       i8    snorm8
```

Greedy-meshed quads produce axis-aligned normals, so the snorm8 encoding is lossless (values are exactly -1, 0, or +1).

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| VP-1 | A vertex at index `draw_metadata[slot].vertex_offset + i` belongs exclusively to chunk `slot` | Mesh rebuild pass writes non-overlapping regions |
| VP-2 | Vertex data for a slot is valid only when `mesh_version[slot] == chunk_version[slot]` | Version check before read; R-4 fallback on mismatch |
| VP-3 | A slot with `chunk_resident_flags[slot] == 0` has undefined vertex pool content in its region | Pool lifecycle |
| VP-4 | Total pool size = `MAX_SLOTS * MAX_VERTS_PER_CHUNK * 16` bytes (Option A fixed reservation) | Buffer creation |

---

## Valid Value Ranges

| Field | Range | Notes |
|---|---|---|
| `position.x/y/z` | `f32`, typically integer-aligned for voxel geometry | World-space; chunk_coord * 64 + local offset |
| `normal_x/y/z` | `-128 .. 127` (i8 snorm) | Greedy mesh normals are axis-aligned: -1, 0, or +1 |
| `material_id` | `0 .. 255` | Index into `material_table`; 0 is typically default |
| `vertex_offset` (in draw_metadata) | `0 .. total_pool_capacity - 1` | Out-of-range = buffer overrun |

---

## Producers

| Producer | When | What it writes |
|---|---|---|
| Greedy mesher (R-1) | Mesh rebuild for dirty chunks | Full vertex region for rebuilt chunk |

Currently runs on CPU (Rust, Web Worker). Future target: GPU compute reading `chunk_occupancy_atlas`, writing directly into `vertex_pool`.

---

## Consumers

| Consumer | Stage | How it reads |
|---|---|---|
| Depth prepass | R-2 | Vertex fetch via `draw_metadata` offsets |
| Occlusion cull (meshlet path) | R-4 phase 2 | `MeshletDesc.vertex_base` references into this pool |
| Main color pass | R-5 | Vertex fetch via `indirect_draw_buf` args (baseVertex) |
| Debug viz | R-9 | Wireframe re-render via same draw path |

---

## Underspecified

> **DESIGN GAP:** The narrative documents ([gpu-chunk-pool](../gpu-chunk-pool.md), [pipeline-stages](../pipeline-stages.md)) do not state concrete values for `MAX_VERTS_PER_CHUNK` or total pool capacity. The mesh pool design section in gpu-chunk-pool.md mentions "65K verts" in a budget example but does not commit to it as a spec value. These must be determined by profiling worst-case greedy mesh output for a fully dense 64-cubed chunk and stated as named constants before buffer allocation can be implemented.
>
> Candidate values (to be validated):
> - `MAX_VERTS_PER_CHUNK`: 65,536 (64K) — upper bound for a fully dense chunk
> - Total pool capacity: `MAX_SLOTS * MAX_VERTS_PER_CHUNK` vertices
> - At 1024 slots and 64K verts: 1024 * 65536 * 16B = ~1 GB (Option A worst case; typical usage far lower)

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Packing roundtrip:** Encode position + normal + material into 16-byte packed vertex, decode, verify exact match for axis-aligned normals.
2. **Region isolation:** After mesh rebuild for slot S, verify no bytes outside `[vertex_offset, vertex_offset + vertex_count)` were modified.
3. **Material ID preservation:** Greedy mesher output material ID matches source palette lookup for each emitted quad.

### Property tests (Rust, randomized)

4. **Offset non-overlap:** For any two resident slots S1, S2, their `[vertex_offset, vertex_offset + vertex_count)` ranges do not overlap.
5. **Capacity bound:** For randomized chunk occupancy, greedy mesher vertex count never exceeds `MAX_VERTS_PER_CHUNK`.

### GPU validation (WGSL compute)

6. **Readback test:** After mesh rebuild, dispatch compute shader that reads vertices via `draw_metadata` offsets, verify positions fall within expected chunk world-space bounds.
7. **Normal axis check:** Verify all normals decode to axis-aligned unit vectors (exactly one component is +/-1, others are 0).

---

## See Also

- [index-pool](index-pool.md) — companion index buffer, same pool pattern
- [chunk-occupancy-atlas](chunk-occupancy-atlas.md) — authoritative source that mesh is derived from
- [gpu-chunk-pool](../gpu-chunk-pool.md) — mesh pool design, Option A vs Option B allocation
- [pipeline-stages](../pipeline-stages.md) — R-1 (mesh rebuild), R-2 (depth prepass), R-5 (color pass)
