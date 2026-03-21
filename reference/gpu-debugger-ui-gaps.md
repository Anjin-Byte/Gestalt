# GPU Debugger UI — State Coverage & Component Gaps

**Type:** reference
**Status:** current
**Date:** 2026-03-20
**Scope:** UI components and views needed to debug and track GPU state at runtime in the Gestalt testbed.

---

## Design Principle

A debugger's job is to display and step through a program's state in a way that is readable and helps a programmer make informed changes. For a GPU-driven renderer, this means surfacing not just *what* the current value of a counter is, but *how it is changing*, *where in the scene it is occurring*, and *why a particular frame behaved differently from the last*.

---

## Categories of GPU State Worth Debugging

### 1. Temporal Counters — Single Value Over Time

The current `DiagCounters` panel shows the *last frame only* — a flat snapshot. A counter like `meshlets_culled` is almost meaningless as a single number. You need to see whether it is stable, trending, or spiking. The same applies to `version_mismatches`, `mesh_rebuilds`, and `cascade_ray_hits`.

**Missing component: Sparkline**
A compact single-counter time-series, ~40px tall. Sits inline next to the current value in a PropRow-like layout. Shares the same circular buffer model as `frameTimeline`. This is the highest-value missing primitive — almost every counter becomes more useful with a trend line beside it.

---

### 2. Buffer Allocation State — Occupancy + Fragmentation

`BarMeter` covers "X of N slots used" but tells you nothing about *where* in the buffer those allocations sit. A buffer with 60% occupancy but 80% fragmentation behaves very differently from a contiguous 60%.

**Missing component: AllocationMap**
A fixed-width row of colored cells, each representing one slot or page. Colors encode: free, occupied, dirty/needs-rebuild, eviction candidate. Like a memory map. Surfaces fragmentation patterns that `BarMeter` cannot show.

---

### 3. Spatial Visibility State — What Is Culled and Why

Chunk-level culling decisions (`chunks_empty_skipped`, Hi-Z occlusion, frustum cull) are aggregated counters. As a counter, they tell you *how many* were culled — nothing about *which* chunks or *why*.

**Missing component: ChunkGrid / VisibilityMap**
A 2D grid where each cell represents a chunk. Color encodes state: visible, frustum-culled, Hi-Z-occluded, empty-skipped, not-yet-built. The spatial analogue of `TimelineCanvas` — temporal vs. spatial. Requires the GPU to write per-chunk visibility output back to CPU. The component itself is a canvas heatmap; the hard part is the readback.

---

### 4. Pass Dependency + Overlap — Pipeline Structure

`TimelineCanvas` shows pass durations as a stacked column — passes appear sequential. On real GPU hardware, some passes can overlap in execution. The stacked model correctly shows wall time but hides pipelining.

**Missing component: WaterfallRow**
A horizontal swimlane showing a single pass as a bar starting at its actual GPU start timestamp and ending at completion. Multiple passes rendered as stacked swimlanes reveals true overlap. Requires GPU start+end timestamps (not just duration), which `timestamp-query` can provide.

---

### 5. Frame-to-Frame Diff — What Changed

When paused, you have a frozen frame. What changed in the frame before a spike? Comparing two `FrameSample` objects or two `DiagCounters` snapshots side-by-side is currently impossible.

**Missing component: DiffRow**
A `PropRow` variant that shows `prev → current` with a delta indicator (`+8 ↑` in warning color). Generated from any two snapshots. Pairs naturally with the existing pause/freeze architecture in `PerformancePanel`.

---

### 6. Structured Flag / Bitfield State

GPU indirect draw arguments, visibility flags, and version counters are often packed as bit fields or small integers with semantic meaning. A raw number (`version_mismatches: 14`) does not tell you *which* chunks are mismatched.

**Missing component: BitField / FlagRow**
A row of labeled 1-bit indicators, color-coded on/off or tri-state. Also useful for pipeline state flags: depth write enabled, backface culling on, stencil active.

---

### 7. Event Log — Why Did That Spike Happen

Counters show *what* happened. A log shows *why* — for example: "chunk 47 evicted due to memory pressure at frame 1203, rebuilt at frame 1205." This is a push model rather than a poll model: the GPU readback path emits events rather than just updating counters.

**Missing component: EventLog**
A scrollable list of timestamped entries with severity (info / warning / error), frame number, and a short message. Capped at N entries with the same circular buffer approach as `frameTimeline`. Fundamentally different from all other debug components because it is event-driven, not frame-sampled.

---

## Summary Table

| State type | Current coverage | Missing component |
|---|---|---|
| Single counter, current value | `PropRow` | — |
| Single counter, over time | None | **Sparkline** |
| Buffer occupancy | `BarMeter` | — |
| Buffer fragmentation | None | **AllocationMap** |
| Per-frame pass timing | `TimelineCanvas` | — |
| Pass overlap / pipeline | None | **WaterfallRow** |
| Spatial chunk visibility | None | **ChunkGrid** |
| Frame-to-frame delta | None | **DiffRow** |
| Bit flags / packed state | None | **BitField / FlagRow** |
| Causal event stream | None | **EventLog** |

---

## Recommended Build Order

Ordered by implementation cost vs. debuggability payoff. The first two require no new GPU-side work — they reuse data already flowing through existing stores.

1. **Sparkline** — reuses `frameTimeline` circular buffer model; pure canvas, no new readback
2. **DiffRow** — variant of `PropRow`; operates on two frozen snapshots already supported by pause architecture
3. **AllocationMap** — canvas heatmap; requires buffer slot occupancy data from GPU readback
4. **EventLog** — new push-model store alongside `frameTimeline`; no spatial data required
5. **WaterfallRow** — requires per-pass start+end timestamps from `timestamp-query`, not just duration
6. **BitField / FlagRow** — simple DOM component; blocked on deciding which flags to expose
7. **ChunkGrid** — highest GPU-side cost; requires per-chunk visibility written back each frame
