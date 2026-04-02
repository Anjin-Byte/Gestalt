# Indirect Draw Pipeline Review

**Type:** reference
**Date:** 2026-03-24
**Files under review:**
- `crates/wasm_renderer/src/shaders/build_indirect.wgsl`
- `crates/wasm_renderer/src/passes/build_indirect.rs`
- `crates/wasm_renderer/src/shaders/build_wireframe.wgsl`
- `crates/wasm_renderer/src/passes/build_wireframe.rs`
- `crates/wasm_renderer/src/pool_gpu.rs` (buffer definitions)
- `crates/wasm_renderer/src/pool.rs` (constants)
- `crates/wasm_renderer/src/lib.rs` (dispatch + draw call sites)

> Critical review of the indirect draw pipeline: how R-1 mesh output flows through build_indirect and build_wireframe compute passes into DrawIndexedIndirect arguments consumed by render passes.

---

## Finding 1: No Slot Count Guard in Either Shader

**Severity:** High
**Type:** Correctness — out-of-bounds write

### Description

Both `build_indirect.wgsl` and `build_wireframe.wgsl` dispatch `ceil(slot_count/64)` workgroups of 64 threads. The shaders use `gid.x` as the slot index but **never check if `gid.x < slot_count`**.

`build_indirect.wgsl` (lines 16-29):
```wgsl
fn main(@builtin(global_invocation_id) gid: vec3u) {
    let slot = gid.x;
    // No bounds check — proceeds to read/write for ALL gid.x values
    let meta_base = slot * DRAW_META_STRIDE;
    let ind_base = slot * INDIRECT_STRIDE;
    let index_count = draw_meta[meta_base + 3u];
    indirect_buf[ind_base] = index_count;
    // ...
}
```

With `slot_count = 1` and `workgroups = ceil(1/64) = 1`, threads 0-63 all execute. Threads 1-63 read from `draw_meta` at offsets 8-504 (slots 1-63) which contain uninitialized or zeroed data, then write to `indirect_buf` at offsets 5-315.

The same issue exists in `build_wireframe.wgsl` (line 23):
```wgsl
fn main(@builtin(global_invocation_id) gid: vec3u) {
    let slot = gid.x;
    // No bounds check
    let meta_base = slot * DRAW_META_STRIDE;
    let tri_index_count = draw_meta[meta_base + 3u];
    // ...
}
```

### Impact

For `slot_count = 1`:
- 63 threads read zeroed `draw_meta` → `index_count = 0`
- They write `instance_count = 0` to indirect args (because `select(0u, 1u, 0u > 0u) = 0`)
- The render loop calls `draw_indexed_indirect` only for `slot < self.resident_count = 1`
- **Net effect: no visible bug** — excess writes produce zero-instance draws that are never consumed

For `slot_count = 1000`:
- `workgroups = ceil(1000/64) = 16` → 1024 threads
- Threads 1000-1023 access `draw_meta` at slots 1000-1023
- These slots exist (MAX_SLOTS = 1024) but contain stale/zeroed data
- They write to `indirect_buf` slots 1000-1023 — harmless if render loop also stops at `resident_count = 1000`

For `slot_count = 1024` (MAX_SLOTS, exact):
- `workgroups = 16` → 1024 threads, all valid
- **No out-of-bounds**

For `slot_count = 1025` (impossible currently, but if MAX_SLOTS increases):
- `workgroups = 17` → 1088 threads
- Threads 1025-1087 access buffer at `slot * STRIDE` which is beyond the allocated buffer size
- **Buffer overflow** — undefined behavior, GPU crash or data corruption

### Root Cause

The shaders have a comment `// Push constant: slot_count` (build_indirect.wgsl line 12) acknowledging the need for a bound, but no push constant was implemented. The slot_count is passed as a dispatch parameter (workgroup count) but not as shader-visible data.

### Recommended Fix

Add an early return:
```wgsl
@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) gid: vec3u) {
    let slot = gid.x;
    if slot >= MAX_SLOTS {
        return;
    }
    // ... rest of shader
}
```

This is safe because `MAX_SLOTS` is a compile-time constant matching the buffer allocation. Alternatively, pass `slot_count` as a push constant or uniform for tighter bounds, but the constant approach is simpler and sufficient.

### Evidence

The dispatch code in `build_indirect.rs` (line 93):
```rust
let workgroups = (slot_count + 63) / 64;
```

This rounds up, creating excess threads. With `slot_count = 1`, 63 excess threads execute.

---

## Finding 2: build_indirect Does Not Clamp Overflowed draw_meta Counts

**Severity:** High
**Type:** Correctness — reads corrupted data

### Description

This finding connects to Finding 3 from the greedy mesher review. The R-1 mesh rebuild shader's overflow guard (`atomicAdd` then `continue`) can leave `draw_meta.index_count` inflated beyond the actual data written. `build_indirect.wgsl` reads this count and writes it directly to the indirect draw args:

```wgsl
let index_count = draw_meta[meta_base + 3u];
indirect_buf[ind_base] = index_count;  // ← unclamped
```

If the R-1 shader overflowed and the counter reached, say, 25000 (beyond MAX_INDICES_PER_CHUNK = 24576), then:
1. `build_indirect` writes `index_count = 25000` to indirect args
2. The render pass issues `draw_indexed_indirect` with 25000 indices
3. Indices 24576-24999 are read from unwritten buffer space (zeros or stale data)
4. These stale indices likely point to vertex 0, producing degenerate triangles at the origin

### Impact

For the current test scene (4416 verts, 6624 indices — well under limits), this cannot trigger. It becomes real when complex scenes approach the per-chunk limits.

### Recommended Fix

Clamp in the shader:
```wgsl
let index_count = min(draw_meta[meta_base + 3u], MAX_INDICES_PER_CHUNK);
```

One line, zero performance cost, prevents garbage geometry from overflowed counters.

The same clamp should be applied in `build_wireframe.wgsl`:
```wgsl
let tri_index_count = min(draw_meta[meta_base + 3u], MAX_INDICES_PER_CHUNK);
```

---

## Finding 3: build_wireframe Serial Loop — Abysmal Parallelism

**Severity:** Medium
**Type:** Performance

### Description

`build_wireframe.wgsl` processes ALL quads for a slot in a serial loop on a single thread:

```wgsl
let quad_count = tri_index_count / 6u;
for (var q = 0u; q < quad_count; q++) {
    // Read 6 indices, write 8 indices
}
```

For the test scene (1104 quads), one thread performs:
- 1104 × 4 = 4416 global memory reads (from `index_pool`)
- 1104 × 8 = 8832 global memory writes (to `wire_index_pool`)
- Total: 13,248 global memory operations on a single thread

Meanwhile, 63 other threads in the workgroup sit idle.

### Comparison to build_indirect

`build_indirect.wgsl` is fine — each thread does 1 read + 5 writes per slot (6 operations total). It's inherently one-slot-per-thread.

`build_wireframe.wgsl` is fundamentally different — it processes a variable-length list of quads per slot. This should be parallelized across threads.

### Quantitative Impact

At ~100 cycles per global memory access (cached):
```
13,248 × 100 = 1,324,800 cycles ≈ 0.88ms at 1.5 GHz
```

This is for ONE chunk. With 100 resident chunks averaging 1000 quads:
```
100 × 1000 × (4+8) × 100 = 120,000,000 cycles ≈ 80ms
```

That's 80ms of serial memory traffic — completely unacceptable. The wireframe build would become the frame bottleneck, not the actual rendering.

### Recommended Fix

Parallelize across quads within each slot. Options:

**Option A: One thread per quad.**
```
dispatch: (slot_count, ceil(max_quads/64), 1)
@workgroup_size(64, 1, 1)

fn main(gid: vec3u) {
    let slot = gid.x;     // which chunk
    let quad = gid.y * 64 + local_id.x;  // which quad
    if quad >= quad_count { return; }
    // Convert this single quad's 6 tri indices → 8 edge indices
}
```

Problem: `quad_count` varies per slot. Would need a per-slot quad count uniform or read from draw_meta.

**Option B: Fixed parallelism with workgroup cooperation.**
Dispatch one workgroup per slot (same as now), but distribute quads across 64 threads:
```wgsl
let quads_per_thread = (quad_count + 63u) / 64u;
let start = local_id.x * quads_per_thread;
let end = min(start + quads_per_thread, quad_count);
for (var q = start; q < end; q++) { ... }
```

This reduces per-thread work by 64× with minimal code change.

**Recommendation:** Option B — simple, effective, no dispatch shape changes.

---

## Finding 4: Wireframe Produces Duplicate Edges at Shared Quad Boundaries

**Severity:** Low
**Type:** Correctness / visual quality

### Description

The wireframe conversion emits 4 edges per quad: `[v0,v1, v1,v2, v2,v3, v3,v0]`. When two quads share an edge (common in greedy mesh output — adjacent quads share exactly one edge), that shared edge is emitted twice: once by each quad.

### Example

Two adjacent +Y quads merged separately:
```
Quad A: v0-v1-v2-v3 (left half)
Quad B: v4-v5-v6-v7 (right half, shares edge v3-v2 = v4-v5)

Quad A edges: [v0,v1], [v1,v2], [v2,v3], [v3,v0]
Quad B edges: [v4,v5], [v5,v6], [v6,v7], [v7,v4]
```

If v3=v4 and v2=v5, then edge `[v2,v3]` from A and `[v4,v5]` from B are the same line drawn twice.

### Impact

- Doubled line overdraw on shared edges (no visual artifact — lines are drawn on top of each other)
- ~30-50% more wireframe indices than necessary for typical greedy meshes
- Increased memory and draw bandwidth for wireframe mode

### How Three.js Handles This

Three.js `WireframeGeometry.js` (lines 79-142) deduplicates edges using a hash set:
```javascript
const keys = new Set();
for each triangle (a, b, c):
  for each edge (a,b), (b,c), (c,a):
    key = `${min(a,b)}_${max(a,b)}`
    if (!keys.has(key)):
      keys.add(key)
      output edge
```

### Recommended Fix (Future)

Edge deduplication on the GPU requires either:
1. **Sorting + compaction** — sort edge pairs, remove consecutive duplicates
2. **Hash map in shared memory** — prohibitively expensive in WGSL
3. **Accept duplicates** — the current approach is correct, just wasteful

For Phase 1, accepting duplicates is fine. Deduplication matters when wireframe is used on dense scenes (Phase 2+). The visual output is correct either way.

---

## Finding 5: base_vertex in Indirect Args Is Written as u32, Interpreted as i32

**Severity:** Medium
**Type:** Correctness — type mismatch

### Description

`build_indirect.wgsl` (line 28):
```wgsl
indirect_buf[ind_base + 3u] = slot * MAX_VERTS_PER_CHUNK;  // base_vertex (i32 reinterp)
```

The comment acknowledges this is an i32 reinterpretation. The `DrawIndexedIndirect` struct in the WebGPU spec defines `baseVertex` as a **signed 32-bit integer (i32)**:

```
struct DrawIndexedIndirect {
    indexCount: u32,
    instanceCount: u32,
    firstIndex: u32,
    baseVertex: i32,   // ← signed
    firstInstance: u32,
};
```

Writing `slot * MAX_VERTS_PER_CHUNK` as a u32 into the i32 field works correctly **as long as the value fits in i32** (≤ 2,147,483,647). Let's check:

```
MAX_SLOTS × MAX_VERTS_PER_CHUNK = 1024 × 16384 = 16,777,216
```

16,777,216 < 2,147,483,647 — fits in i32. **No bug for current constants.**

### When It Breaks

If MAX_SLOTS increases to 131072 (128K slots):
```
131072 × 16384 = 2,147,483,648 = 2^31 → i32 overflow
```

At this point, `base_vertex` wraps negative, producing incorrect vertex fetches.

### Impact

Not a current issue. Becomes relevant if the pool scales significantly.

### Recommended Fix

Use `bitcast<i32>()` explicitly to document the type boundary:
```wgsl
indirect_buf[ind_base + 3u] = bitcast<u32>(i32(slot * MAX_VERTS_PER_CHUNK));
```

Or add a static assertion in `pool.rs`:
```rust
const _: () = assert!(
    (MAX_SLOTS as u64 * MAX_VERTS_PER_CHUNK as u64) <= i32::MAX as u64,
    "base_vertex must fit in i32 for DrawIndexedIndirect"
);
```

The same applies to `build_wireframe.wgsl` line 62.

---

## Finding 6: Dispatch Timing — Compute and Render in Separate Submissions

**Severity:** Medium
**Type:** Architecture — unnecessary serialization

### Description

In `load_test_scene()` (lib.rs lines 271-287), the compute passes are submitted in a single batch:
```rust
let mut encoder = ...;
self.mesh_pass.dispatch(&mut encoder, ...);         // R-1
self.build_indirect_pass.dispatch(&mut encoder, ...); // build_indirect
self.build_wireframe_pass.dispatch(&mut encoder, ...); // build_wireframe
self.queue.submit(std::iter::once(encoder.finish()));
```

This is correct for load-time — all three compute passes in one submission, GPU executes them in order.

However, in `render_frame()` (lib.rs lines 347-488), the render passes are in a **separate** `queue.submit()` call from the compute passes. Since compute was dispatched at load time (not per-frame), this is fine for the current single-load architecture.

### The Problem Emerges with Per-Frame Rebuilds

When dirty chunks trigger per-frame R-1 dispatches (Phase 4 edit protocol), the flow becomes:

```
Frame N:
  1. Check dirty chunks
  2. Dispatch R-1 for dirty slots (compute)
  3. Dispatch build_indirect (compute)
  4. Dispatch build_wireframe (compute)
  5. queue.submit(compute_encoder)
  6. begin_render_pass(R-2 depth) — reads indirect_buf
  7. begin_render_pass(R-5 color) — reads indirect_buf
  8. queue.submit(render_encoder)
```

Steps 5 and 8 are separate submissions. WebGPU guarantees in-order execution within a queue, so compute (step 5) completes before render (step 8). **This is correct but suboptimal.** Separate submissions prevent the GPU from overlapping compute and render work from the same frame.

### Recommended Fix

Encode compute and render into the **same** command encoder:
```rust
let mut encoder = ...;
// Compute
if has_dirty {
    self.mesh_pass.dispatch(&mut encoder, ...);
    self.build_indirect_pass.dispatch(&mut encoder, ...);
}
// Render (same encoder)
encoder.begin_render_pass(R-2);
encoder.begin_render_pass(R-5);
queue.submit(std::iter::once(encoder.finish()));
```

This allows the GPU to pipeline compute→render without a submission boundary. The render pass implicitly waits for compute to write the indirect buffer (WebGPU guarantees execution ordering within an encoder).

---

## Finding 7: build_indirect Reads draw_meta as Raw u32 Array, Not Struct

**Severity:** Low
**Type:** Fragility / maintenance

### Description

Both shaders access `draw_meta` as `array<u32>` and hardcode the field offsets:

```wgsl
let index_count = draw_meta[meta_base + 3u];  // offset 3 = index_count field
```

The `DrawMeta` struct (pool.rs) is:
```rust
pub struct DrawMeta {
    pub vertex_offset: u32,   // offset 0
    pub vertex_count: u32,    // offset 1
    pub index_offset: u32,    // offset 2
    pub index_count: u32,     // offset 3
    pub material_base: u32,   // offset 4
    pub _pad: [u32; 3],       // offset 5-7
}
```

The WGSL shaders hardcode `meta_base + 3u` to access `index_count`. If the struct layout changes (e.g., a field is added before `index_count`), the shaders silently read the wrong field with no compilation error.

### Recommended Fix

Define the `DrawMeta` struct in WGSL and access by field name:

```wgsl
struct DrawMeta {
    vertex_offset: u32,
    vertex_count: u32,
    index_offset: u32,
    index_count: u32,
    material_base: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var<storage, read> draw_meta: array<DrawMeta>;

// Access:
let meta = draw_meta[slot];
let index_count = meta.index_count;  // ← field name, not offset
```

This requires changing the buffer binding from `array<u32>` to `array<DrawMeta>`, but is safer and self-documenting.

Note: `draw_meta` in `mesh_rebuild.wgsl` uses `array<atomic<u32>>` because R-1 needs atomicAdd. The indirect/wireframe shaders read after R-1 completes, so they can use the struct form.

---

## Finding 8: Wireframe Buffer Memory — 128 MB for Edge Indices

**Severity:** Medium
**Type:** Resource waste

### Description

`pool.rs` (lines 85-87):
```rust
pub const MAX_WIRE_INDICES_PER_CHUNK: u32 = MAX_INDICES_PER_CHUNK / 6 * 8; // = 32768
pub const TOTAL_WIRE_INDEX_BYTES: u64 = MAX_WIRE_INDICES_PER_CHUNK as u64
    * INDEX_BYTES as u64 * MAX_SLOTS as u64;
```

Calculation:
```
32768 indices × 4 bytes × 1024 slots = 134,217,728 bytes = 128 MB
```

Plus the wireframe indirect buffer: `20 bytes × 1024 slots = 20 KB` (negligible).

**128 MB is allocated at pool creation for wireframe edge indices** — a debug visualization feature. This is the same order of magnitude as the primary index pool (96 MB) and is allocated unconditionally, regardless of whether wireframe mode is ever activated.

### Impact

The total GPU memory budget increases from ~385 MB to ~513 MB. On a 2 GB mobile GPU, this is 25% of total VRAM consumed by a debug-only feature.

### Recommended Fix

**Option A: Lazy allocation.** Create the wireframe buffer only when wireframe mode is first activated. Destroy it when switching away.

**Option B: Shared/reuse.** The wireframe index buffer is only needed during wireframe mode rendering. It could share memory with another buffer that isn't needed during wireframe mode (e.g., if depth viz mode uses the same allocation slot).

**Option C: Reduce capacity.** Wireframe is a debug tool. Limit to 64 or 128 slots instead of 1024. If the user tries to wireframe-view a scene with >128 chunks, only the first 128 are wireframed. This reduces the allocation from 128 MB to 8-16 MB.

**Option D: Accept it.** 128 MB is large but tolerable on desktop GPUs with 4+ GB VRAM. The cost is real only on memory-constrained devices.

---

## Finding 9: draw_all_slots Loops on CPU Instead of Using Multi-Draw

**Severity:** Low
**Type:** Performance — missed optimization

### Description

`lib.rs` (lines 494-500):
```rust
fn draw_all_slots(&self, pass: &mut wgpu::RenderPass<'_>) {
    let indirect_buf = self.pool.indirect_buffer();
    for slot in 0..self.resident_count {
        pass.draw_indexed_indirect(indirect_buf, slot as u64 * 20);
    }
}
```

This issues one `draw_indexed_indirect` call per resident slot. For 1024 resident chunks, this is 1024 CPU-side calls, each encoding a GPU command.

### What Would Be Better

WebGPU has `multi_draw_indexed_indirect` (available via the `multi-draw-indirect` extension):
```rust
pass.multi_draw_indexed_indirect(indirect_buf, 0, resident_count);
```

One CPU call, one GPU command, all slots drawn. Reduces CPU overhead from O(n) to O(1).

### Why It's Not Critical

wgpu v29 may not expose `multi_draw_indexed_indirect` on all backends. The WebGPU spec includes it as an optional feature. For 1024 slots, the overhead of 1024 `draw_indexed_indirect` calls is ~0.1ms — measurable but not a bottleneck.

### Recommended Fix

Check if the feature is available at device creation:
```rust
if device.features().contains(wgpu::Features::MULTI_DRAW_INDIRECT) {
    pass.multi_draw_indexed_indirect(indirect_buf, 0, resident_count);
} else {
    for slot in 0..self.resident_count {
        pass.draw_indexed_indirect(indirect_buf, slot as u64 * 20);
    }
}
```

Deferred to Phase 2+ when slot counts increase.

---

## Finding 10: No Validation That build_indirect Runs Before Render

**Severity:** Low
**Type:** Architecture — implicit ordering dependency

### Description

The render passes in `render_frame()` consume `indirect_draw_buf` via `draw_indexed_indirect`. This buffer is written by `build_indirect_pass`, dispatched in `load_test_scene()`. There is no explicit synchronization between the compute write and the render read — it relies on WebGPU's implicit queue ordering (earlier submissions complete before later ones).

### Why It Works Now

`load_test_scene()` submits the compute encoder before `render_frame()` is ever called. WebGPU guarantees that `queue.submit(A)` completes before `queue.submit(B)` on the same queue. The indirect buffer is therefore ready by the first `render_frame()`.

### Why It Could Break

If `render_frame()` is called before `load_test_scene()` completes (e.g., due to an async race in the initialization flow), the render pass reads an empty/stale indirect buffer. With `instance_count = 0` in the zeroed buffer, nothing would draw — a silent black frame, not a crash.

More critically: when per-frame R-1 rebuilds are added (Phase 4), the compute and render passes must be in the same submission or explicitly ordered. The current architecture (separate submissions) is technically correct but fragile.

### Recommended Fix

Document the ordering dependency as a code comment. When Phase 4 adds per-frame rebuilds, ensure compute and render are in the same command encoder (see Finding 6).

---

## Finding 11: build_indirect Ignores vertex_count and vertex_offset

**Severity:** Low
**Type:** Incomplete — future-proofing

### Description

`build_indirect.wgsl` reads only `index_count` from `draw_meta`:
```wgsl
let index_count = draw_meta[meta_base + 3u];
```

It ignores `vertex_count` (offset 1), `vertex_offset` (offset 0), and `index_offset` (offset 2). Instead, it computes `first_index` and `base_vertex` from slot position:

```wgsl
indirect_buf[ind_base + 2u] = slot * MAX_INDICES_PER_CHUNK;  // first_index
indirect_buf[ind_base + 3u] = slot * MAX_VERTS_PER_CHUNK;    // base_vertex
```

This assumes fixed-size allocation per slot (each slot gets MAX_VERTS and MAX_INDICES reserved). The `vertex_offset` and `index_offset` fields in DrawMeta (written by R-1 as 0) are never consulted.

### Impact

This is correct for the current fixed-allocation pool. When variable-allocation is implemented (Phase 6 streaming), `vertex_offset` and `index_offset` will contain non-zero values indicating where this slot's data actually starts in the shared pool. `build_indirect` must then use these offsets:

```wgsl
let first_index = draw_meta[meta_base + 2u];  // index_offset from R-1
let base_vertex = draw_meta[meta_base + 0u];  // vertex_offset from R-1
```

### Recommended Fix

No change needed now. Flag as a migration point for Phase 6.

---

## Summary Table

| # | Finding | Severity | Type | Action |
|---|---------|----------|------|--------|
| 1 | No slot count guard in either shader | High | OOB write | Fix now — add `if slot >= MAX_SLOTS { return; }` |
| 2 | build_indirect doesn't clamp overflowed counts | High | Corrupt data | Fix now — `min(count, MAX)` |
| 3 | build_wireframe serial loop (0% parallelism) | Medium | Performance | Fix Phase 2 — distribute quads across threads |
| 4 | Wireframe emits duplicate shared edges | Low | Visual quality | Accept for Phase 1, deduplicate Phase 2+ |
| 5 | base_vertex written as u32, spec is i32 | Medium | Type mismatch | Fix now — add static assertion in pool.rs |
| 6 | Compute and render in separate submissions | Medium | Perf / architecture | Fix Phase 4 — single encoder for compute+render |
| 7 | draw_meta accessed as raw u32, not struct | Low | Fragility | Fix Phase 2 — use WGSL struct type |
| 8 | 128 MB wireframe buffer allocated unconditionally | Medium | Resource waste | Fix Phase 2 — lazy allocation |
| 9 | Per-slot CPU loop instead of multi_draw_indirect | Low | Performance | Fix Phase 2+ — feature-gate multi-draw |
| 10 | No explicit validation of compute→render ordering | Low | Architecture | Document now, fix Phase 4 |
| 11 | build_indirect ignores vertex/index offsets | Low | Incomplete | Correct for fixed alloc, flag for Phase 6 |

---

## Data Flow Diagram

```
R-1 Mesh Rebuild (compute)
  │
  ├─→ vertex_pool[]     (vertices at slot * MAX_VERTS * 4)
  ├─→ index_pool[]      (indices at slot * MAX_INDICES)
  └─→ draw_meta[]       (vertex_count, index_count via atomicAdd)
         │
         ├─→ build_indirect (compute, 1 thread/slot)
         │     └─→ indirect_draw_buf[]  (DrawIndexedIndirect × 1024)
         │           │
         │           ├─→ R-2 depth prepass:  draw_indexed_indirect(buf, slot*20)
         │           └─→ R-5 color pass:     draw_indexed_indirect(buf, slot*20)
         │
         └─→ build_wireframe (compute, 1 thread/slot, SERIAL per-quad loop)
               ├─→ wire_index_pool[]    (edge indices, 8 per quad)
               └─→ wire_indirect_buf[]  (DrawIndexedIndirect × 1024)
                     │
                     └─→ R-9 wireframe:  draw_indexed_indirect(buf, slot*20)
                           (LineList topology)

Buffer sizes:
  indirect_draw_buf:    20 KB  (1024 × 20 B)
  wire_indirect_buf:    20 KB  (1024 × 20 B)
  wire_index_pool:     128 MB  (1024 × 32768 × 4 B) ← Finding 8
  index_pool:           96 MB  (1024 × 24576 × 4 B)
  vertex_pool:         256 MB  (1024 × 16384 × 16 B)
  draw_meta:            32 KB  (1024 × 32 B)
```
