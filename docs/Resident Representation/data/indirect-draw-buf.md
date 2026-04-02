# Indirect Draw Buffer

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Per-frame transient — populated by R-4 occlusion cull, consumed by R-5 drawIndexedIndirect.

> The indirect draw argument buffer. Contains one DrawIndexedIndirect entry per surviving meshlet (or chunk fallback). The bridge between GPU-driven culling and GPU-driven drawing.

---

## Identity

- **Buffer name:** `indirect_draw_buf`
- **WGSL type:** `array<DrawIndexedIndirectArgs>` (see layout below)
- **GPU usage:** `STORAGE | INDIRECT`
- **Binding:** `STORAGE` for R-4 compute writes; `INDIRECT` for R-5 `drawIndexedIndirect` consumption

---

## Layout

Each entry follows the WebGPU `DrawIndexedIndirect` struct layout (matches the GPUDrawIndexedIndirectArgs spec):

```
struct DrawIndexedIndirectArgs {
    index_count:    u32,   // byte offset 0  — number of indices to draw
    instance_count: u32,   // byte offset 4  — number of instances (always 1)
    first_index:    u32,   // byte offset 8  — offset into index_pool (in index units)
    base_vertex:    i32,   // byte offset 12 — added to each index before vertex fetch
    first_instance: u32,   // byte offset 16 — first instance ID (always 0)
}
// Total: 20 bytes per entry
```

The buffer also includes an atomic counter at a fixed location for the number of surviving draws:

```
Buffer layout:
  [0 .. 3]                                    draw_count: u32 (atomic)
  [4 .. 4 + MAX_DRAWS * 20 - 1]              draw_args: array<DrawIndexedIndirectArgs>

  Alternatively (if draw_count is a separate buffer):
  indirect_draw_buf:   array<DrawIndexedIndirectArgs, MAX_DRAWS>
  draw_count_buf:      u32  (separate buffer for multiDrawIndexedIndirect count)
```

### Entry Semantics

Each entry describes one draw call:

- **Meshlet-level entry** (normal path): emitted when `meshlet_version[slot] == chunk_version[slot]`. Draws a single meshlet's triangles.
  ```
  index_count    = meshlet_desc.index_count
  instance_count = 1
  first_index    = meshlet_desc.index_offset    // into index_pool
  base_vertex    = meshlet_desc.vertex_base     // into vertex_pool
  first_instance = 0
  ```

- **Chunk-level fallback entry**: emitted when meshlets are stale (`meshlet_version[slot] != chunk_version[slot]`). Draws the entire chunk mesh.
  ```
  index_count    = draw_metadata[slot].index_count
  instance_count = 1
  first_index    = draw_metadata[slot].index_offset
  base_vertex    = draw_metadata[slot].vertex_offset
  first_instance = 0
  ```

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| ID-1 | Every entry references valid regions within `vertex_pool` and `index_pool` | R-4 reads offsets from `draw_metadata` or `meshlet_desc_pool`, which are validated at mesh/meshlet build time |
| ID-2 | `instance_count` is always 1; `first_instance` is always 0 | R-4 writes constant values |
| ID-3 | `draw_count` accurately reflects the number of valid entries | R-4 uses atomic increment; R-5 uses draw_count to bound multi-draw |
| ID-4 | Buffer is fully written by R-4 before R-5 reads it | Pipeline barrier between R-4 compute and R-5 indirect draw |
| ID-5 | Buffer contents are undefined at frame start; R-4 overwrites from index 0 | R-4 resets draw_count to 0 at start of pass |
| ID-6 | No entry references a non-resident slot | R-4 phase 1 filters on `chunk_resident_flags` before any entry can be emitted |
| ID-7 | Total entries never exceed `MAX_DRAWS` | Atomic counter saturates or R-4 bounds-checks before write |

---

## Valid Value Ranges

| Field | Range | Notes |
|---|---|---|
| `index_count` | `0 .. MAX_INDICES_PER_CHUNK` | 0 would be a no-op draw; typically > 0 since empty chunks are culled |
| `instance_count` | `1` | Fixed; no instancing in this pipeline |
| `first_index` | `0 .. total_index_pool_capacity - 1` | Must be within index_pool bounds |
| `base_vertex` | `0 .. total_vertex_pool_capacity - 1` | Signed i32 per spec, but always non-negative in this pipeline |
| `first_instance` | `0` | Fixed |
| `draw_count` | `0 .. MAX_DRAWS` | 0 = nothing visible (camera inside solid geometry or fully occluded) |

---

## Producers

| Producer | When | What it writes |
|---|---|---|
| R-4 phase 1 (chunk coarse cull) | Every frame | Populates `chunk_visible_list` — does NOT write to indirect_draw_buf directly |
| R-4 phase 2 (meshlet fine cull) | Every frame, after phase 1 | Writes `DrawIndexedIndirectArgs` entries and increments `draw_count` atomically |

### R-4 Write Protocol

```
// At frame start (or R-4 preamble):
atomicStore(&draw_count, 0u)

// Phase 2, per surviving meshlet or chunk fallback:
let idx = atomicAdd(&draw_count, 1u)
if idx < MAX_DRAWS {
    indirect_draw_buf[idx] = DrawIndexedIndirectArgs { ... }
}
```

The atomic counter guarantees entries are packed contiguously from index 0. Entry ordering is non-deterministic (depends on GPU thread scheduling), but draw order does not affect correctness because the depth prepass (R-2) has already populated the depth buffer.

---

## Consumers

| Consumer | Stage | How it reads |
|---|---|---|
| Main color pass | R-5 | `drawIndexedIndirect` or `multiDrawIndexedIndirect` consuming `draw_count` entries |
| Debug visualization | R-9 | May re-issue draws for wireframe overlay using same buffer |

### R-5 Consumption

```wgsl
// Pseudocode for R-5 draw submission:
for i in 0 .. draw_count:
    drawIndexedIndirect(indirect_draw_buf, offset = i * 20)
```

Or, if `multiDrawIndexedIndirect` is available:

```wgsl
multiDrawIndexedIndirect(indirect_draw_buf, offset = 0, draw_count)
```

---

## Underspecified

> **DESIGN GAP:** The narrative documents ([pipeline-stages](../pipeline-stages.md), [meshlets](../meshlets.md)) do not state a concrete value for `MAX_DRAWS`. This constant determines the buffer allocation size and the saturation bound for the atomic counter.
>
> Candidate values (to be validated):
> - With chunk-only culling (no meshlets): `MAX_DRAWS = MAX_SLOTS` (e.g., 1024)
> - With meshlet culling (Option S, 512 meshlets per chunk): `MAX_DRAWS = MAX_SLOTS * 512` (e.g., 524,288)
> - Buffer size at 524K draws: 524288 * 20 = ~10 MB
> - In practice, most draws are culled. The buffer must be sized for the worst case (all meshlets in all chunks visible), but typical occupancy will be a fraction of capacity.
>
> `MAX_DRAWS` must be stated as a named constant. The atomic counter saturation check (`if idx < MAX_DRAWS`) is a correctness requirement — overflow would write out of bounds.

---

## Testing Strategy

### Unit tests (CPU-side)

1. **Struct layout:** Verify `DrawIndexedIndirectArgs` is exactly 20 bytes with fields at correct byte offsets (matches WebGPU spec).
2. **Atomic counter reset:** Verify draw_count is 0 at start of R-4.

### GPU validation (WGSL compute)

3. **Readback after R-4:** Render a known scene, read back indirect_draw_buf and draw_count, verify draw_count matches expected number of visible chunks/meshlets.
4. **Entry validity:** For each entry in readback, verify `first_index + index_count` does not exceed index_pool capacity and `base_vertex` does not exceed vertex_pool capacity.
5. **Full occlusion:** Place camera inside a box, verify draw_count == number of box-interior faces visible (near zero for a sealed box viewed from inside with backface culling).
6. **No draw on empty scene:** Load zero chunks, verify draw_count == 0.

### Integration tests

7. **Cull correctness:** Place 100 chunks, fully occlude 50 behind a wall, verify draw_count is approximately 50 (plus/minus conservative bias).
8. **Fallback path:** Invalidate meshlets for one chunk (set meshlet_version != chunk_version), verify R-4 emits one chunk-level fallback entry with correct draw_metadata offsets.
9. **Saturation guard:** Artificially set MAX_DRAWS to a small value, verify no out-of-bounds writes occur when more meshlets pass culling than MAX_DRAWS allows.

---

## See Also

- [vertex-pool](vertex-pool.md) — vertex data referenced by `base_vertex`
- [index-pool](index-pool.md) — index data referenced by `first_index` and `index_count`
- [hiz-pyramid](hiz-pyramid.md) — the occlusion test data that determines which entries are emitted
- [depth-texture](depth-texture.md) — depth prepass output that enables correct draw ordering
- [pipeline-stages](../pipeline-stages.md) — R-4 (cull), R-5 (color pass)
- [meshlets](../meshlets.md) — two-phase cull design, fallback behavior
