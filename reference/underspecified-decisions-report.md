# Underspecified Design Decisions — Analysis Report

**Type:** reference
**Status:** current
**Date:** 2026-03-22
**Purpose:** Resolve 8 underspecified items flagged during the strict specification pass. Each section provides time/space analysis, UX impact assessment, and a recommended approach.

---

## 1. Chunk Palette: Fixed vs Variable Allocation

### The Decision

How to allocate GPU memory for per-chunk material palettes. Each chunk has 1–256 unique MaterialIds stored as u16 values packed 2 per u32.

### Option A: Fixed 512 Bytes Per Slot

Every slot gets the maximum 256 entries (512 bytes) regardless of actual palette size.

| Metric | Value |
|---|---|
| Per-slot cost | 512 B (always) |
| 4,096 slots | 2 MB |
| 16,384 slots | 8 MB |
| 65,536 slots | 32 MB |
| Addressing | `slot_offset = slot_index * 256` — O(1), branch-free |
| Allocation | None — preallocated at pool creation |
| Deallocation | None — slot reuse is implicit |

**Time complexity:** O(1) for all operations. No allocation, no compaction, no fragmentation.

**Space waste:** A typical single-material chunk (1 palette entry) wastes 510 of 512 bytes. For a scene with 90% single-material chunks and 4,096 slots: ~1.8 MB wasted. Compare to occupancy atlas at 32 KB/slot = 128 MB — the palette waste is 1.4% of the atlas cost.

### Option B: Variable Allocation with Freelist

Each slot allocates only what it needs. A freelist tracks available regions in a shared palette buffer.

| Metric | Value |
|---|---|
| Per-slot cost | 2 × palette_size bytes |
| Addressing | `slot_offset = palette_offset_table[slot_index]` — O(1) with indirection |
| Allocation | Freelist search — O(N) worst case, O(1) amortized with free list |
| Deallocation | Return to freelist — O(1) |
| Compaction | Needed periodically to defragment — O(N_slots) |

**Time complexity:** Allocation is O(1) amortized but O(N) worst-case when fragmented. Compaction stalls the pipeline.

**Space savings:** For 4,096 slots with 90% single-material: ~2 MB → ~0.2 MB. Savings of 1.8 MB.

### UX Impact

**None perceptible.** Palette access happens deep in the GPU pipeline (R-5 fragment shader, R-6 hit test). The user never sees palette allocation. The only UX-relevant scenario: if variable allocation causes a stall during compaction while the user is editing, they'd see a frame hitch. Fixed allocation eliminates this entirely.

### Recommendation

**Fixed 512 bytes per slot.** The memory cost is trivial relative to the occupancy atlas. The simplicity guarantees zero frame hitches from allocation pressure. The O(1) branch-free addressing is faster on GPU where divergent memory access patterns are expensive.

---

## 2. Dirty Chunks: Reset Timing

### The Decision

When to clear the `dirty_chunks` bitset to all zeros — before edit kernels run, or after propagation completes.

### Option A: Reset at Frame Start

```
Frame N:
  1. Reset dirty_chunks to 0
  2. Edit kernels run → set bits via atomicOr
  3. Propagation pass reads dirty_chunks → writes stale flags
  4. Compaction pass reads stale flags → writes rebuild queues
  5. R-1/I-3 rebuilds run
```

**Time:** O(N_words) to memset — 32 u32 for 1024 slots = 128 bytes. Negligible.

**Correctness:** All edits within a frame accumulate into the same dirty set. Propagation sees them all at once. No race: edit kernels use atomicOr (idempotent for already-set bits), propagation runs after all edits are submitted.

### Option B: Reset After Propagation

```
Frame N:
  1. Edit kernels run → set bits (accumulated since last reset)
  2. Propagation pass reads dirty_chunks → writes stale flags
  3. Reset dirty_chunks to 0
  4. Compaction + rebuilds
```

**Correctness risk:** If edits from frame N-1 weren't fully propagated (budget exceeded), their dirty bits would persist into frame N's propagation. This is a feature if you want carryover, but it makes reasoning about "which frame's edits are we processing?" harder.

### UX Impact

**Option A is more predictable.** The user edits voxels, the results appear next frame. No ambiguity about which edits have been processed. Option B could cause an edit from frame N-1 to visually appear in frame N+1 if propagation was budget-limited — the user sees a 2-frame delay instead of 1-frame, which feels sluggish for interactive editing.

### Recommendation

**Option A: Reset at frame start.** Simpler mental model (this frame's edits → this frame's propagation → this frame's rebuilds), more responsive editing (max 1-frame latency), trivial implementation cost.

---

## 3. Rebuild Queues: Unconsumed Entry Persistence

### The Decision

When R-1 (mesh rebuild) only has budget to process 50 of 100 stale chunks, what happens to the remaining 50?

### Option A: Re-compact Every Frame

The queue is ephemeral. Every frame:
1. Compaction pass scans `stale_mesh` bitset → builds fresh `mesh_rebuild_queue`
2. R-1 processes `queue[0..budget]`
3. R-1 clears `stale_mesh` bits for processed slots only
4. Next frame: compaction scans again, finds the remaining 50 stale slots, rebuilds the queue

**Time:** Compaction is O(N_slots / 32) — one bitwise scan over the stale bitset. For 4,096 slots: 128 u32 words. On GPU, one workgroup of 64 threads handles this in microseconds.

**Space:** Queue is MAX_SLOTS × 4 bytes = 16 KB. Allocated once, overwritten each frame.

### Option B: Persistent Queue with Tail Pointer

The queue persists across frames. R-1 advances a tail pointer.
1. Compaction appends new stale slots to the end of the queue
2. R-1 consumes from the head up to budget
3. Head pointer advances; unconsumed entries remain

**Time:** Compaction is O(newly_stale) per frame (only appends new entries). But must handle: What if a slot is edited again while still in the queue? The queue entry becomes stale-of-stale. Need deduplication or version checking.

**Space:** Same queue buffer, but with head/tail pointers (8 extra bytes).

**Complexity:** Need to handle queue wrapping, deduplication of re-edited slots, and the case where the queue fills up (all slots stale, none rebuilt yet).

### UX Impact

**Option A: more responsive after large edits.** If the user makes a huge edit that dirties 200 chunks, then makes a small edit next frame, Option A rebuilds the most important chunks fresh each frame (compaction can prioritize by camera distance). Option B processes in FIFO order — the small edit waits behind the 200-chunk backlog regardless of relevance.

**Option B: more predictable throughput.** Every stale chunk eventually gets rebuilt in order. No chunk is perpetually re-queued because higher-priority chunks keep jumping ahead.

### Recommendation

**Option A: Re-compact every frame.** The compaction cost is negligible. The ability to re-prioritize each frame (by camera distance, by visible vs. off-screen) is worth more than the predictability of FIFO. Users doing interactive sculpting care about the chunks they're looking at updating first, not about fairness.

---

## 4. Cascade Atlas: Texture Array

### The Decision

How to store the cascade atlases (one per cascade level, typically 4 levels).

### Option A: Single 2D Texture with Manual Atlas Packing

All cascade levels packed into one large 2D texture. Shader computes UV offsets per level.

| Metric | Value |
|---|---|
| Bindings | 1 texture, 1 sampler |
| Addressing | `uv = base_uv + level_offset` — manual offset math in every shader |
| Waste | Packing 4 different-resolution levels into a rectangle wastes ~20% due to padding |
| Mip support | None (custom atlas, not hardware mips) |

### Option B: Texture Array (One Layer Per Level)

`texture_2d_array<rgba16f>` with `array_length = N_CASCADE_LEVELS`. Each layer is the same dimensions (constant atlas size property from Sannikov).

| Metric | Value |
|---|---|
| Bindings | 1 texture array, 1 sampler |
| Addressing | `textureLoad(atlas, uv, layer)` — layer index = cascade level |
| Waste | Zero — each layer is exactly the atlas size |
| Hardware support | Texture arrays are natively supported in WebGPU |

**Memory:** Identical for both options. 4 levels × atlas_size × 8 bytes (rgba16f) ≈ 64 MB at 1080p.

### UX Impact

**None.** This is purely a GPU implementation detail. The user sees the same GI quality regardless. The only indirect UX impact: if manual atlas packing has a bug (off-by-one in level offset), it produces visual artifacts. Texture arrays eliminate this class of bug.

### Recommendation

**Texture array.** Zero waste, cleaner addressing, eliminates offset bugs, same binding count. The constant-atlas-size property of radiance cascades means every layer has identical dimensions — texture arrays are purpose-built for this.

---

## 5. R-8 GI Composite: Inline vs Separate Pass

### The Decision

Where the GI contribution from radiance cascades gets added to the final pixel color.

### Variant A: Inline in R-5 Fragment Shader

```
Pipeline ordering: R-6 → R-7 → R-5
```

R-5's fragment shader samples `cascade_atlas_0` directly. It has natural access to the material's albedo (already computed for the base color), normal (from vertex data), and world position (from depth + inverse projection).

| Metric | Value |
|---|---|
| Extra passes | 0 |
| Extra render targets | 0 |
| Pipeline serialization | R-7 must complete before R-5 starts |
| Albedo access | Direct — already in the fragment shader |
| Physical correctness | Full: `final = albedo * (ambient + GI_irradiance)` |
| Shader complexity | R-5 fragment adds ~10 lines (cascade sample + modulation) |
| Memory bandwidth | One extra texture read per fragment (cascade atlas) |

**Serialization cost:** R-6 (cascade build) + R-7 (cascade merge) must finish before R-5 starts. Cascade build is the heaviest: ~1.8 ms for 4 levels at 1080p (from pipeline-stages.md). Cascade merge is ~0.9 ms. Total: R-5 starts ~2.7 ms after R-2 finishes.

Without cascades (placeholder ambient), R-5 starts immediately after R-4 (~0.5 ms after R-2). So cascades add ~2.2 ms of serialized latency to the color pass start.

**But:** R-5 itself runs for ~4.2 ms. The total frame time goes from `R-2(0.5) + R-3(0.2) + R-4(0.3) + R-5(4.2) = 5.2 ms` to `R-2(0.5) + R-6(1.8) + R-7(0.9) + R-3/R-4(overlap with R-6, ~0) + R-5(4.5) = 7.7 ms`. Still well under 16.6 ms (60 fps).

### Variant B: Separate Fullscreen Pass

```
Pipeline ordering: R-5 and R-6/R-7 run in parallel, then R-8 composites
```

R-5 writes albedo-only color. R-6/R-7 build cascades in parallel. R-8 is a fullscreen quad that reads both and composites.

| Metric | Value |
|---|---|
| Extra passes | 1 fullscreen quad |
| Extra render targets | 1 (G-buffer for albedo, or accept incorrect additive blend) |
| Pipeline serialization | R-5 and R-6/R-7 can overlap |
| Albedo access | Requires G-buffer or color_target re-read |
| Physical correctness | Only with G-buffer; additive blend is incorrect |
| Shader complexity | R-8 pass + G-buffer management |
| Memory bandwidth | G-buffer write (R-5) + G-buffer read (R-8) + cascade read (R-8) |

**Overlap savings:** R-5 (4.2 ms) and R-6+R-7 (2.7 ms) can run in parallel on GPUs with enough compute units. Theoretical savings: 2.7 ms. But WebGPU doesn't expose async compute queues — all work goes through one queue. The driver may overlap internally, but we can't guarantee it.

**G-buffer cost:** Writing albedo to a separate render target in R-5 adds ~0.5 ms bandwidth. Reading it back in R-8 adds another ~0.3 ms. Net: overlap saves 2.7 ms, G-buffer costs 0.8 ms, net savings ~1.9 ms — but only if the driver actually overlaps, which is not guaranteed.

### UX Impact

**Variant A: simpler, guaranteed correct lighting.** The user sees physically correct GI from frame 1. No visual artifacts from incorrect blending.

**Variant B: potentially smoother at high resolution** if the driver overlaps compute and render. But if it doesn't (common in WebGPU today), the extra G-buffer cost makes it *slower* than Variant A, and without a G-buffer, the lighting is incorrect (additive instead of multiplicative).

**The real UX question:** can the user tell the difference between 7.7 ms/frame and 5.8 ms/frame? Both are well above 60 fps. The answer is no — both feel equally responsive. The only scenario where it matters: 4K resolution, where R-5 might take 16+ ms and the cascade serialization pushes the frame over budget. At that point, you'd want Variant B. But that's a future optimization, not an MVP decision.

### Recommendation

**Variant A: inline in R-5.** Correct lighting, simpler implementation, no G-buffer, works well within 60 fps budget. If 4K performance becomes a problem, Variant B can be added later as a toggle without changing the data structures.

---

## 6. Pool Capacities: MAX_VERTS, MAX_INDICES, MAX_DRAWS

### The Data (from greedy mesher analysis)

For a 64³ chunk (62³ usable interior) with binary greedy meshing:

| Scenario | Quads | Vertices (4/quad) | Indices (6/quad) | Vertex bytes (16 B) | Index bytes (4 B) |
|---|---|---|---|---|---|
| **Worst case** (3D checkerboard) | ~119,000 | ~477,000 | ~716,000 | 7.3 MB | 2.7 MB |
| **Worst realistic** (noisy surface) | ~20,000 | ~80,000 | ~120,000 | 1.2 MB | 480 KB |
| **Typical** (solid terrain) | 500–2,000 | 2,000–8,000 | 3,000–12,000 | 32–128 KB | 12–48 KB |
| **Minimal** (solid cube) | 6 | 24 | 36 | 384 B | 144 B |

The theoretical maximum (119K quads) is a degenerate 3D checkerboard that never occurs in real content. The worst *realistic* case is a highly noisy surface (e.g., procedural terrain with per-voxel noise) which produces ~20K quads.

### Decision: Fixed vs Variable Pool

**Option A: Fixed worst-case allocation per slot**

Set `MAX_VERTS_PER_CHUNK = 131,072` (next power of 2 above 119K × 4) and `MAX_INDICES_PER_CHUNK = 786,432` (119K × 6, rounded).

| Slots | Vertex pool | Index pool | Total |
|---|---|---|---|
| 1,024 | 2 GB | 3 GB | 5 GB |
| 4,096 | 8 GB | 12 GB | 20 GB |

**This doesn't work.** Even at 1,024 slots, 5 GB for mesh data alone exceeds most GPU memory budgets.

**Option B: Fixed realistic-worst-case allocation per slot**

Set `MAX_VERTS_PER_CHUNK = 98,304` (next multiple of 1024 above 80K) and `MAX_INDICES_PER_CHUNK = 147,456`.

| Slots | Vertex pool | Index pool | Total |
|---|---|---|---|
| 1,024 | 1.5 GB | 576 MB | ~2 GB |
| 4,096 | 6 GB | 2.3 GB | ~8 GB |

Still too much for 4K+ slots. Only viable at ≤1,024 slots.

**Option C: Fixed generous-typical allocation + overflow handling**

Set `MAX_VERTS_PER_CHUNK = 16,384` and `MAX_INDICES_PER_CHUNK = 24,576`. This covers 99%+ of real chunks. Chunks that exceed the limit fall back to unmerged face rendering (one quad per exposed face — more triangles but guaranteed to fit in a smaller buffer) or split into multiple draw calls.

| Slots | Vertex pool | Index pool | Total |
|---|---|---|---|
| 1,024 | 256 MB | 96 MB | 352 MB |
| 4,096 | 1 GB | 384 MB | 1.4 GB |
| 16,384 | 4 GB | 1.5 GB | 5.5 GB |

Manageable up to 4,096 slots. Above that, need variable allocation.

**Option D: Variable allocation with a pool allocator (suballocation)**

A single large vertex buffer + index buffer. Each chunk gets a contiguous region. A GPU-side or CPU-side freelist manages allocation. `draw_metadata[slot].vertex_offset/index_offset` point into the shared pool.

| Metric | Fixed (Option C) | Variable (Option D) |
|---|---|---|
| Memory efficiency | ~10-20% utilization (typical chunks use 1-5% of their allocation) | 90%+ utilization |
| Fragmentation | None | Yes — external fragmentation from variable-size regions |
| Allocation speed | O(1) — multiply | O(1) amortized with free list, O(N) worst case |
| Compaction needed | Never | Periodically (stalls pipeline) |
| Implementation | Trivial | Moderate (pool allocator) |

### MAX_DRAWS

This is simpler. `MAX_DRAWS` = maximum number of indirect draw calls per frame = maximum number of non-culled chunks.

For 1,024 slots: MAX_DRAWS = 1,024 (every resident chunk visible). 1,024 × 20 bytes = 20 KB. Trivial.

For 4,096 slots: MAX_DRAWS = 4,096. 80 KB. Still trivial.

For meshlet-level draws: MAX_DRAWS = MAX_SLOTS × 512 (max meshlets per chunk). 4,096 × 512 = 2M draws × 20 bytes = 40 MB. This matters — but meshlet draws are a Phase 2 optimization.

### UX Impact

**Memory determines how many chunks can be resident**, which directly determines visual quality:
- Fewer resident chunks = more pop-in as the camera moves (chunks load/evict frequently)
- More resident chunks = smoother exploration, less streaming latency

**Option C (fixed generous) at 4,096 slots** keeps ~1.4 GB for mesh data. On a 4 GB GPU (low-end discrete), this leaves 2.6 GB for everything else (textures, occupancy atlas, cascades). On an 8+ GB GPU, it's comfortable.

**Option D (variable)** uses memory proportional to actual content. A scene with 4,096 mostly-solid chunks might use only 200 MB instead of 1.4 GB. But the compaction stall can cause a visible hitch if it happens during camera movement.

### Recommendation

**Start with Option C (fixed generous) for the demo.** `MAX_VERTS_PER_CHUNK = 16,384`, `MAX_INDICES_PER_CHUNK = 24,576`. This is simple, predictable, and sufficient for the demo scene (solid room + sphere). Add overflow fallback (unmerged face rendering) for the rare chunk that exceeds the limit.

**MAX_DRAWS = MAX_SLOTS** for chunk-level draws. Revisit when meshlets are implemented.

**Migrate to Option D (variable) when streaming is implemented** — at that point you're already managing slot lifetimes and the pool allocator is a natural extension.

### Concrete Constants

```
MAX_SLOTS              = 4096        (configurable; demo starts at 1024)
MAX_VERTS_PER_CHUNK    = 16384       (16K verts × 16 B = 256 KB per slot)
MAX_INDICES_PER_CHUNK  = 24576       (24K indices × 4 B = 96 KB per slot)
MAX_DRAWS              = MAX_SLOTS   (4096 for chunk-level; revisit for meshlets)

vertex_pool_size       = MAX_SLOTS × MAX_VERTS_PER_CHUNK × 16  = 1 GB at 4096 slots
index_pool_size        = MAX_SLOTS × MAX_INDICES_PER_CHUNK × 4 = 384 MB at 4096 slots
indirect_draw_buf_size = MAX_DRAWS × 20                         = 80 KB
```

---

## Summary

| Item | Decision | Why | UX impact |
|---|---|---|---|
| 1. Palette allocation | Fixed 512 B/slot | O(1), no fragmentation, trivial memory | None |
| 2. Dirty reset timing | Frame start | Predictable 1-frame edit latency | Better responsiveness |
| 3. Rebuild queues | Re-compact every frame | Enables priority-based rebuild order | Camera-facing chunks update first |
| 4. Cascade atlas | Texture array | Clean addressing, no offset bugs | None (same visual quality) |
| 5. R-8 GI composite | Inline in R-5 | Correct lighting, no G-buffer, simple | Correct GI from day 1 |
| 6. Pool capacities | Fixed generous + overflow | Simple, predictable, sufficient for demo | Determines max resident chunks |

All decisions favor simplicity and correctness over theoretical optimality. The variable/advanced approaches (variable palette, persistent queues, separate GI pass, variable pool) are future optimizations that can be added without changing the data model.
