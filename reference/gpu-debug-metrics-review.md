# GPU Debugging, Metrics, and Phi Exposure Review

**Type:** reference
**Date:** 2026-03-24
**Scope:** End-to-end audit of GPU state instrumentation, metrics generation, data flow to UI, and Phi component utilization.

> Traces the full path from GPU buffer state through WASM getters, TypeScript stores, to Phi panel display. Identifies dead code, broken pipelines, fake data, and unused components.

---

## Finding 1: Frame Timing Data Is Fabricated

**Severity:** High
**Type:** Misleading data

### Description

`RendererController.ts` (lines 57-63) pushes frame timing to the `frameTimeline` store with hardcoded pass ratios:

```typescript
frameTimeline.push({
  totalMs,
  passes: {
    "R-2 Depth Prepass": totalMs * 0.15,
    "R-5 Color Pass": totalMs * 0.85,
  },
});
```

The `totalMs` value is real (measured via `performance.now()` around `render_frame()`), but the per-pass breakdown is a fixed 15%/85% split that has no relationship to actual GPU execution time.

### Impact

- The `TimelineCanvas` renders a visually compelling stacked bar chart where R-2 is always exactly 15% and R-5 is always exactly 85% of every frame. This looks like real data but contains no signal.
- The `PassBreakdownTable` computes averages and standard deviations of these fake ratios, producing precise-looking statistics that are meaningless.
- A developer using these panels to diagnose a performance issue would draw wrong conclusions — if R-1 compute becomes the bottleneck, it's invisible because it's not in the pass list at all.

### What's Missing

Five compute passes execute per scene load but are never timed:
- I-3 Summary Rebuild
- R-1 Greedy Mesh Rebuild
- build_indirect
- build_wireframe
- (Future: R-3 Hi-Z, R-4 Occlusion Cull, R-6/R-7 Cascades)

### Root Cause

No GPU timestamp query infrastructure exists. wgpu supports `QueryType::Timestamp` for GPU-side pass timing, but no query pools are created. The `performance.now()` measurement captures CPU-side encoding + submit time, not GPU execution time. There is no way to split this into per-pass measurements without timestamp queries.

### Recommended Fix

**Phase 1 (immediate):** Measure CPU-side encoding time per pass by wrapping each `begin_render_pass`/`end` section in `performance.now()` calls on the WASM side. This captures encoding cost, not GPU cost, but is at least real data rather than fabricated ratios.

**Phase 2 (proper):** Implement wgpu timestamp query pools:
1. Create a `QuerySet` with `QueryType::Timestamp` and 2 × num_passes entries
2. Write timestamps at pass begin/end via `write_timestamp()`
3. Resolve queries into a readback buffer after submit
4. Async readback the results next frame
5. Push real per-pass GPU nanoseconds to `frameTimeline`

### Evidence

`timeline.ts` defines `PASS_COLORS` for 8 passes (lines 12-21), proving the UI was designed for real multi-pass timing. The `TimelineCanvas` and `PassBreakdownTable` visualization components are fully functional and would immediately display correct data if the source were real.

---

## Finding 2: Diagnostic Counter Store Is Dead

**Severity:** High
**Type:** Dead pipeline

### Description

The `timeline.ts` store defines `DiagCounters` with 6 GPU diagnostic counters (lines 54-61):

```typescript
interface DiagCounters {
  meshlets_culled: number;
  chunks_empty_skipped: number;
  version_mismatches: number;
  summary_rebuilds: number;
  mesh_rebuilds: number;
  cascade_ray_hits: number;
}
```

The store has `update()` and `clear()` methods. The `PerformancePanel` subscribes to `diagCounters` (line 29) and `diagHistory` (line 34), derives 6 sparkline arrays (lines 38-43), and renders 6 `CounterRow` components (lines 106-111) with danger thresholds.

**The store is never updated.** `diagCounters.update()` is called from exactly zero locations in the codebase. All 6 counters display "—" permanently.

### Impact

- The PerformancePanel's bottom section (GPU DIAGNOSTICS) is entirely non-functional
- 6 `CounterRow` components with sparkline history arrays are rendered every frame for nothing
- The `diagHistory` store accumulates an empty array, wasting memory
- A developer looking at the PerformancePanel sees an impressive-looking diagnostics section that has never shown a single real value

### What Data Could Flow Here

| Counter | Source | Available Now? |
|---|---|---|
| `meshlets_culled` | R-4b meshlet cull pass | No — meshlets not implemented (Phase 5) |
| `chunks_empty_skipped` | R-2/R-5 skip logic using chunk_flags.is_empty | Partially — flags exist, skip logic doesn't count |
| `version_mismatches` | R-1 mesh rebuild version check | No — version checking not implemented |
| `summary_rebuilds` | I-3 dispatch count per frame | Yes — `resident_count` at dispatch time |
| `mesh_rebuilds` | R-1 dispatch count per frame | Yes — `resident_count` at dispatch time |
| `cascade_ray_hits` | R-6 cascade build atomic counter | No — GI not implemented (Phase 3) |

Of the 6 counters, only `summary_rebuilds` and `mesh_rebuilds` could be populated now (they equal the number of dirty chunks dispatched, which is currently always 0 after initial load since no edits occur).

### Recommended Fix

**Immediate:** Expose dispatch counts from WASM:
```rust
pub fn get_summary_rebuild_count(&self) -> u32 { ... }
pub fn get_mesh_rebuild_count(&self) -> u32 { ... }
```

Call from RendererController, push to `diagCounters.update()`.

**Deferred (Phase 3+):** Wire GPU atomic counters for meshlets_culled, cascade_ray_hits when those passes are implemented.

**Deferred (Phase 4):** Wire version_mismatches when the edit protocol tracks version conflicts.

---

## Finding 3: debug_readback.rs Is Dead Code

**Severity:** Medium
**Type:** Dead code

### Description

`debug_readback.rs` (117 lines) implements a complete GPU readback infrastructure:
- `DebugReadback` struct with 3 staging buffers (meta, vertex, index)
- `copy_slot_to_staging()` — records GPU-to-staging copy commands
- `read_draw_meta_sync()` — maps staging buffer, returns `DrawMeta` struct
- `log_comparison()` — logs CPU vs GPU vertex/index count comparison

The struct is instantiated in `Renderer::new()` (lib.rs line 145) and stored as a field on the Renderer. It is never used after construction.

### Why It's Dead

The original plan was to call readback in `load_test_scene()` to validate GPU mesh output against the CPU reference. This was attempted but `device.poll(Wait)` is a no-op on the WebGPU WASM backend — synchronous buffer mapping doesn't work. The readback code was deferred and the CPU reference counts were used directly instead.

### Impact

- 3 staging buffers are allocated at init (~262 KB + ~98 KB + 32 bytes = ~360 KB of GPU memory) and never used
- The `DebugReadback` struct takes space in the Renderer struct for no purpose
- Developers see the module and assume readback works, when it doesn't

### Recommended Fix

**Option A: Remove entirely.** Delete the module, remove the field from Renderer, reclaim ~360 KB GPU memory. Re-implement when async readback is properly solved.

**Option B: Make it async.** Use `wasm_bindgen_futures::spawn_local` with a Promise-based readback flow. The mapped buffer results would be posted to a callback. This is the correct approach for WASM WebGPU but requires careful lifecycle management.

**Option C: Keep as dormant.** Leave the code but document it as non-functional. Mark it `#[allow(dead_code)]` with a comment explaining why. This preserves the infrastructure for when async readback is implemented.

### Evidence

Searching for `debug_readback` usage:
- `lib.rs:145` — `debug_readback: debug_readback::DebugReadback::new(&device)` (construction)
- `lib.rs:44` — field declaration
- No other call sites in the entire codebase

---

## Finding 4: Protocol Ring Buffer and Snapshot Are Vestigial

**Severity:** Low
**Type:** Dead code / architectural debt

### Description

`protocol.ts` (lines 81-127) defines two SharedArrayBuffer layouts:

**Ring Buffer** (17,288 bytes):
- 240-slot circular buffer for per-frame timing
- Each slot: totalMs (f32) + passCount (u32) + 8 × (nameHash:u32 + ms:f32) = 72 bytes
- Header: head index + capacity

**Snapshot Buffer** (6,152 bytes):
- Version + chunk count header
- Up to 1024 chunk entries (chunkId:u32 + slotIndex:u16)

Both were designed for the worker-based architecture (ADR-0013) where the WASM renderer in the worker would write directly to SharedArrayBuffers, and the main thread would read them lock-free.

### Why They're Dead

ADR-0014 moved all GPU work to the main thread. There is no Web Worker. SharedArrayBuffers are no longer needed for cross-thread communication. Stats flow directly from WASM getters to Svelte stores within the same thread.

The constants are still imported by `RendererController.ts` (indirectly via the protocol module), but no SharedArrayBuffer is ever created or written to.

### Impact

- ~200 lines of protocol definition for unused binary layouts
- Developers may attempt to use the ring buffer API, not realizing it was designed for a deleted architecture
- COOP/COEP headers (required for SharedArrayBuffer) are no longer needed — simplifies deployment

### Recommended Fix

**Option A: Archive to reference/deprecated-protocol.ts.** Keep the binary layout definitions as reference for a future worker architecture, but remove from the active codebase.

**Option B: Slim protocol.ts.** Keep only the message type interfaces (`StatsMessage`, `FrameTimingMessage`, `RendererStats`) that are still used by stores. Remove ring buffer, snapshot, command opcodes, and binary encoding constants.

**Option C: Leave as-is.** The dead code doesn't affect runtime. Just adds cognitive load for readers.

---

## Finding 5: InspectorPanel Shows Static GPU Pool Data

**Severity:** Medium
**Type:** Misleading data

### Description

The GPU POOL section of `InspectorPanel.svelte` (lines 73-78) displays hardcoded static strings:

```svelte
<Section sectionId="inspector-pool" title="GPU POOL">
  <PropRow label="Occupancy Atlas" value="32 MB" />
  <PropRow label="Vertex Pool" value="256 MB" />
  <PropRow label="Index Pool" value="96 MB" />
  <PropRow label="Total Allocated" value="384 MB" />
</Section>
```

These values are the theoretical maximum sizes (from `pool.rs` constants), not actual GPU memory consumption. The values are correct for MAX_SLOTS=1024, but:

1. The actual GPU may allocate less if the driver over-provisions or aligns differently
2. There's no indication of how much of each pool is actually in use vs. reserved
3. The numbers don't change if MAX_SLOTS changes (they're hardcoded strings, not derived from constants)

### Impact

A user sees "384 MB" and believes that much GPU memory is in use. In reality, most of it is empty (1 chunk out of 1024 slots is resident). The useful metric is "how full is the pool" not "how big is the pool."

### Recommended Fix

Replace hardcoded strings with computed values from WASM:

```rust
// New WASM getters
pub fn get_pool_total_bytes(&self) -> u64 { ... } // sum of all buffer sizes
pub fn get_pool_used_verts(&self) -> u32 { self.mesh_verts }
pub fn get_pool_used_indices(&self) -> u32 { self.mesh_indices }
pub fn get_free_slot_count(&self) -> u32 { self.pool.allocator().free_count() }
```

Display as BarMeters:
```svelte
<BarMeter label="Slots" value={residentCount} max={1024} />
<BarMeter label="Vertex Pool" value={meshVerts * 16} max={256 * 1024 * 1024} unit="B" />
<BarMeter label="Index Pool" value={meshIndices * 4} max={96 * 1024 * 1024} unit="B" />
```

---

## Finding 6: Camera Data Flows But Is Not Displayed Usefully

**Severity:** Low
**Type:** Underutilized data

### Description

The `rendererStatsStore` receives camera position and direction every 10 frames. The InspectorPanel displays them as formatted strings:

```svelte
const camPos = $derived(stats?.cameraPos
  ? `(${stats.cameraPos[0].toFixed(1)}, ${stats.cameraPos[1].toFixed(1)}, ...)`
  : "—");
```

The FOV, near plane, and far plane are hardcoded strings ("45°", "0.1 / 2000").

### Impact

- Camera position updates live during orbit — this works correctly
- FOV/near/far are static and don't reflect any runtime changes (though they don't change currently)
- No visual indicator of camera frustum or view direction

### Recommended Fix

Low priority. The current display is functional. If the camera gains adjustable FOV (Phase 2 zoom), expose `get_camera_fov()` and `get_camera_near_far()` from WASM.

---

## Finding 7: Phi Components — 8 Advanced Controls Unused in Production

**Severity:** Low
**Type:** Underutilization

### Description

Phi exports 15 main components. The real panels (Inspector, Performance) use 6. The DemoPanel uses all 15 as a component showcase. The remaining 8 are available but have no real data source:

| Component | Used in Real UI? | Used in DemoPanel? | Potential Real Use |
|---|---|---|---|
| `Slider` | No | Yes | Exposure, light scale, frame budget |
| `ScrubField` | No | Yes | Numeric parameter editing |
| `ToggleGroup` | No | Yes | Render mode (alternative to SelectField) |
| `CheckboxRow` | No | Yes | Enable/disable debug features |
| `StatusIndicator` | No | Yes | Pipeline health (green/yellow/red) |
| `DiffRow` | No | Yes | CPU vs GPU count comparison |
| `BitField` | No | Yes | Chunk flags (is_empty, is_resident, stale_mesh) |
| `ContextMenu` | No | Yes | Right-click actions on chunks |

### Impact

These components were built and tested (166+ tests across the TreeList/ContextMenu system alone) but serve no production purpose. They exist only as DemoPanel showcases.

### Recommended Uses (Post Phase 1.5)

| Component | Concrete Use Case |
|---|---|
| `StatusIndicator` | Show pipeline health: green = running, yellow = rebuilding, red = error |
| `DiffRow` | CPU reference vs GPU mesh counts (when readback works) |
| `BitField` | Per-chunk flags: `is_empty`, `is_resident`, `stale_mesh`, `stale_summary`, `has_emissive` |
| `CheckboxRow` | Toggle debug overlays: AABB wireframes, bricklet grid, chunk boundaries |
| `Slider` | Frame budget line position in TimelineCanvas |
| `ToggleGroup` | Render mode selector (more visual than SelectField dropdown) |

---

## Finding 8: Mesh Stats Come From CPU Reference, Not GPU

**Severity:** Medium
**Type:** Data accuracy

### Description

The mesh statistics displayed in the InspectorPanel (quads, vertices, indices) are computed by the CPU reference mesher (`mesh_cpu.rs`) during `load_test_scene()` and stored as Renderer struct fields. They are NOT read back from the GPU after R-1 compute dispatch.

```rust
// lib.rs lines 296-311 — CPU reference, not GPU readback
let cpu_result = mesh_cpu::mesh_rebuild_cpu(...);
self.mesh_verts += cpu_result.draw_meta.vertex_count;
self.mesh_indices += cpu_result.draw_meta.index_count;
self.mesh_quads += cpu_result.quad_count;
```

### Impact

If the GPU greedy mesher produces different results than the CPU reference (due to a shader bug, floating point divergence, or the issues documented in the greedy mesher review), the UI would show incorrect counts. The user would see "correct" numbers while the actual rendered geometry has a different vertex/index count.

This is especially concerning given:
- Finding 3 from the greedy mesher review (overflow guard leaks counter space)
- The fact that the GPU shader has never been validated against the CPU reference via readback

### Recommended Fix

**Immediate (defensive):** Add a note to the InspectorPanel indicating these are CPU reference values, not GPU-verified:

```svelte
<PropRow label="Vertices (CPU ref)" value={verts} />
```

**Phase 2 (proper):** Implement async GPU readback of `draw_meta` per slot. Compare CPU vs GPU counts. Display both in the panel using `DiffRow`:

```svelte
<DiffRow label="Vertices" prev={cpuVerts} current={gpuVerts} />
```

If they disagree, `StatusIndicator` turns red.

---

## Finding 9: PerformancePanel Is Fully Wired But Receives Degraded Data

**Severity:** Medium
**Type:** Architecture gap

### Description

The PerformancePanel (`PerformancePanel.svelte`, 132 lines) is the most sophisticated Phi panel in the application:

- `TimelineCanvas` — canvas-based 240-frame waterfall chart with hover inspection
- `PassBreakdownTable` — per-pass avg/stddev with color-coded budget warnings
- Frame statistics — last/avg/peak/p50/p95/p99
- Pause/resume control for timeline inspection
- 6 `CounterRow` components with sparkline histories and danger thresholds

**Data quality received:**
- `frameTimeline` — receives real `totalMs` but fake pass breakdown (15/85 split)
- `diagCounters` — receives null (never updated)
- `diagHistory` — receives empty array (never pushed to)

**Data quality displayed:**
- Frame total: **real** (useful)
- Pass breakdown: **fabricated** (misleading)
- Frame percentiles: **real** (useful, derived from real totalMs)
- GPU diagnostics: **"—"** across all 6 counters (dead)

### Impact

The panel gives a false sense of profiling capability. The stacked timeline bars look authoritative but contain no information about where GPU time is actually spent. The diagnostics section is entirely non-functional.

Despite this, the panel infrastructure is correct and complete. If real data were fed to the stores, the UI would immediately display it correctly with no code changes to the panel itself.

### Recommended Fix

Same as Findings 1 and 2 — the panel is not the problem. The data sources are.

---

## Finding 10: No Per-Chunk Introspection

**Severity:** Low
**Type:** Missing feature

### Description

The current stats are aggregated across all chunks: total voxels, total vertices, total indices. There is no way to inspect an individual chunk's state, mesh quality, or occupancy.

### What Would Be Useful

- Per-slot occupancy popcount (how many voxels in this chunk)
- Per-slot vertex/index count (from DrawMeta readback)
- Per-slot flags (is_empty, stale_mesh, has_emissive — from chunk_flags buffer)
- Per-slot AABB (from chunk_aabb buffer)
- Chunk coordinate → slot mapping (from SlotAllocator)

### Phi Component Fit

`TreeList` was built for exactly this use case — a hierarchical list with inline columns, sorting, and per-row status indicators. A "Chunk Inspector" using TreeList would show:

```
Chunk (0,0,0)  │ Slot 0 │ 29505 vox │ 4416 verts │ ● resident
Chunk (1,0,0)  │ Slot 1 │ 0 vox     │ 0 verts    │ ○ empty
...
```

With inline `StatusCell` for flags and `InlineSparkCell` for vertex count history.

### Recommended Fix

Deferred to Phase 2+ when multiple chunks are resident. Currently (1 chunk), aggregated stats are sufficient. The TreeList infrastructure exists and is tested (166 tests) — it needs a data source, not new components.

---

## Summary Table

| # | Finding | Severity | Type | Action |
|---|---------|----------|------|--------|
| 1 | Frame timing data fabricated (15/85 split) | High | Misleading data | Replace with per-pass CPU encoding time (immediate), GPU timestamps (Phase 2) |
| 2 | DiagCounters store is dead (6 counters, 0 data) | High | Dead pipeline | Wire summary_rebuild and mesh_rebuild counts (immediate) |
| 3 | debug_readback.rs is dead code (~360 KB wasted) | Medium | Dead code | Remove or make async (Phase 2) |
| 4 | Protocol ring buffer/snapshot are vestigial | Low | Dead code | Archive or slim protocol.ts |
| 5 | GPU Pool section shows hardcoded static values | Medium | Misleading data | Replace with computed BarMeters |
| 6 | Camera data flows but FOV/near/far hardcoded | Low | Underutilized | Expose from WASM when adjustable |
| 7 | 8 Phi components unused in production | Low | Underutilization | Map to real use cases post Phase 1.5 |
| 8 | Mesh stats from CPU reference, not GPU | Medium | Data accuracy | Add readback validation, label as CPU ref |
| 9 | PerformancePanel fully wired, degraded data | Medium | Architecture gap | Fix data sources (Findings 1+2) |
| 10 | No per-chunk introspection | Low | Missing feature | TreeList chunk inspector (Phase 2+) |

---

## End-to-End Data Flow Diagram

```
GPU Buffers                    WASM Renderer                 RendererController.ts
─────────────                  ──────────────                 ─────────────────────
occupancy_atlas ─┐
vertex_pool     ─┤   ──→   Renderer struct          ──→   get_mesh_verts()
index_pool      ─┤         - mesh_verts (CPU ref)          get_mesh_indices()
draw_meta_buf   ─┘         - mesh_quads (CPU ref)          get_mesh_quads()
                           - resident_count                 get_resident_count()
                           - render_mode                    get_render_mode()
                           - frame_index                    get_frame_index()
                           - camera (pos, dir)              get_camera_pos/dir()

                           NOT exposed:                     NOT available:
                           - draw_meta GPU values            - per-pass timing
                           - chunk_flags                     - diag counters
                           - per-slot vertex counts          - per-chunk breakdown
                           - pool occupancy                  - GPU readback data
                           - debug_readback results

                                                            RendererController
Svelte Stores                 Phi Panels                    ──→ rendererStatsStore
─────────────                 ──────────                         (every 10 frames)
rendererStatsStore  ──→  InspectorPanel                    ──→ frameTimeline
  - frame                  - render mode selector               (every frame,
  - residentCount          - scene stats                         FAKE 15/85 split)
  - meshVerts              - mesh stats
  - cameraPos              - camera display                ──→ diagCounters
                           - static pool display                (NEVER UPDATED)

frameTimeline       ──→  PerformancePanel
  - totalMs (real)         - TimelineCanvas (works, fake data)
  - passes (fake)          - PassBreakdownTable (works, fake data)
                           - frame percentiles (works, real data)
diagCounters (null) ──→    - 6 CounterRows (all show "—")
diagHistory ([])    ──→    - 6 Sparkline histories (empty)
```
