# Debug, Profiling, and Testing

**Type:** spec
**Status:** current
**Date:** 2026-03-21

GPU timing infrastructure, internal state visibility, diagnostic counters,
and the testing strategy for a GPU-resident architecture where shaders are
outside the reach of `cargo test`.

Related: [pipeline-stages](pipeline-stages.md), [edit-protocol](edit-protocol.md), [gpu-chunk-pool](gpu-chunk-pool.md).

---

## The Core Problem

The architecture pushes logic GPU-side intentionally. The same move that
eliminates CPU readbacks and sync points also eliminates the ability to
`println!` your way through a bug. Three separate problems need distinct
solutions:

| Problem | Approach |
|---|---|
| **Performance** — is each pass meeting its budget? | Timestamp queries + pass timeline visualization |
| **State visibility** — what is the GPU actually holding? | Debug render modes + diagnostic counter readback |
| **Correctness** — is the logic right, especially the protocols? | Layered test strategy: native wgpu → CPU reference → protocol invariants → headless browser |

These are distinct tools for distinct failure modes. Conflating them leads to
profilers that don't catch bugs and tests that don't catch regressions.

---

## Pass Timing

### Timestamp Query Infrastructure

WebGPU exposes GPU-side timestamps via the `timestamp-query` optional feature.
When available, timestamps are written directly by the GPU at pass boundaries
— no CPU involvement, no measurement contamination.

```typescript
// Request the feature at device creation
const device = await adapter.requestDevice({
    requiredFeatures: ["timestamp-query"],
});
const hasTimestamps = device.features.has("timestamp-query");
```

`timestamp-query` is available in Chrome (behind `#enable-webgpu-developer-features`
flag or via Origin Trial) and in native wgpu. Firefox and Safari support varies.
The system must degrade gracefully when the feature is absent.

### Query Set Layout

One `GPUQuerySet` per frame. Each pass gets two slots: begin and end.

```typescript
const PASSES = [
    "I-3 Summary Rebuild",
    "R-2 Depth Prepass",
    "R-3 Hi-Z Pyramid",
    "R-4a Chunk Cull",
    "R-4b Meshlet Cull",
    "R-5 Color Pass",
    "R-6 Cascade Build",
    "R-7 Cascade Merge",
] as const;

const querySet = device.createQuerySet({
    type: "timestamp",
    count: PASSES.length * 2,   // begin + end per pass
});

// Resolve buffer (GPU-side, not mappable)
const resolveBuffer = device.createBuffer({
    size: PASSES.length * 2 * 8,   // 8 bytes per timestamp (u64 nanoseconds)
    usage: GPUBufferUsage.QUERY_RESOLVE | GPUBufferUsage.COPY_SRC,
});

// Readback buffer (CPU-mappable)
const readbackBuffer = device.createBuffer({
    size: PASSES.length * 2 * 8,
    usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ,
});
```

### Per-Pass Attachment

Each pass descriptor receives `timestampWrites`:

```typescript
// Compute pass example (R-4a)
const chunkCullPass = encoder.beginComputePass({
    timestampWrites: {
        querySet,
        beginningOfPassWriteIndex: passIndex * 2,
        endOfPassWriteIndex:       passIndex * 2 + 1,
    },
});

// Render pass example (R-5)
const colorPass = encoder.beginRenderPass({
    colorAttachments: [...],
    timestampWrites: {
        querySet,
        beginningOfPassWriteIndex: passIndex * 2,
        endOfPassWriteIndex:       passIndex * 2 + 1,
    },
});
```

### Readback Sequence

At the end of the frame's command encoder, before `submit`:

```typescript
encoder.resolveQuerySet(querySet, 0, PASSES.length * 2, resolveBuffer, 0);
encoder.copyBufferToBuffer(resolveBuffer, 0, readbackBuffer, 0, readbackBuffer.size);
device.queue.submit([encoder.finish()]);

// Async readback — does not stall the render loop
readbackBuffer.mapAsync(GPUMapMode.READ).then(() => {
    const data = new BigInt64Array(readbackBuffer.getMappedRange());
    const durations: Record<string, number> = {};
    for (let i = 0; i < PASSES.length; i++) {
        const begin = data[i * 2];
        const end   = data[i * 2 + 1];
        durations[PASSES[i]] = Number(end - begin) / 1e6;   // nanoseconds → ms
    }
    readbackBuffer.unmap();
    frameTimeline.push(durations);
});
```

The readback is async and arrives 1–3 frames late. This is correct — do not
block on it.

### CPU Fallback

When `timestamp-query` is unavailable, wrap each submit boundary with
`performance.now()` for frame-level granularity. Pass-level breakdown is not
available without the feature. The visualization degrades to a single total-frame
bar rather than per-pass breakdown.

---

## Pass Timeline Visualization

The scrolling stacked timing chart you are describing is a **GPU frame waterfall**.
Build it as a `<canvas>` overlay rendered by `requestAnimationFrame`, fed by the
async timestamp readback.

### Data Model

```typescript
const HISTORY_FRAMES = 240;   // 4 seconds at 60fps

interface FrameSample {
    totalMs: number;
    passes: Record<string, number>;   // pass name → duration ms
}

const history: FrameSample[] = [];   // circular buffer, oldest first
```

### Rendering Logic

Each frame column = one frame. The x-axis is time (scrolling left). The y-axis
is duration in ms. Each pass is drawn as a stacked colored rectangle. The
aggregate height at any column = total frame time for that frame.

```typescript
function drawTimeline(ctx: CanvasRenderingContext2D, history: FrameSample[]) {
    const W = ctx.canvas.width;
    const H = ctx.canvas.height;
    const colW = W / HISTORY_FRAMES;
    const msScale = H / 33.3;   // 33.3ms = 30fps budget line = full height

    ctx.clearRect(0, 0, W, H);

    // Budget lines
    ctx.strokeStyle = "rgba(255,255,0,0.3)";
    ctx.beginPath(); ctx.moveTo(0, H - 16.67 * msScale); ctx.lineTo(W, H - 16.67 * msScale); ctx.stroke();  // 60fps
    ctx.beginPath(); ctx.moveTo(0, H - 33.33 * msScale); ctx.lineTo(W, H - 33.33 * msScale); ctx.stroke();  // 30fps

    for (let f = 0; f < history.length; f++) {
        const x = (HISTORY_FRAMES - history.length + f) * colW;
        let y = H;
        for (const [pass, ms] of Object.entries(history[f].passes)) {
            const h = ms * msScale;
            ctx.fillStyle = PASS_COLORS[pass] ?? "#888";
            ctx.fillRect(x, y - h, colW - 1, h);
            y -= h;
        }
    }

    // Legend — ordered by average duration descending
    const avgByPass = computeAverages(history);
    const sorted = [...avgByPass.entries()].sort((a, b) => b[1] - a[1]);
    sorted.forEach(([pass, avg], i) => {
        ctx.fillStyle = PASS_COLORS[pass] ?? "#888";
        ctx.fillRect(W - 160, 8 + i * 18, 12, 12);
        ctx.fillStyle = "white";
        ctx.fillText(`${pass}  ${avg.toFixed(2)}ms`, W - 144, 18 + i * 18);
    });
}
```

Feed `frameTimeline.push(durations)` in the readback callback,
call `drawTimeline` in the render loop. The legend ordering (by average duration)
surfaces the dominant passes immediately.

---

## State Visibility

### Debug Render Modes

A single `debug_mode: u32` uniform switches the R-5 fragment shader between
normal output and internal data visualization. All modes reuse existing
buffers — no new GPU data is allocated.

```wgsl
// In the R-5 fragment shader (or a parallel fullscreen compute pass)
switch debug_mode {
    case 0u: { /* normal shading */ }
    case 1u: { /* occupancy_summary bricklet heat map */ }
    case 2u: { /* has_emissive tint overlay */ }
    case 3u: { /* chunk_version as hue (recently edited = red) */ }
    case 4u: { /* meshlet cluster color bands */ }
    default: {}
}
```

| Mode | What it shows | Buffers read |
|---|---|---|
| 0 (normal) | Full PBR color | `material_table`, `cascade_atlas_0` |
| 1 (bricklet) | `occupancy_summary` — gray = empty bricklet, colored = occupied; reveals traversal skip quality | `occupancy_summary` |
| 2 (emissive) | Green tint on `has_emissive` chunks; confirms I-3 emissive scan is correct | `chunk_flags` |
| 3 (version) | `chunk_version` mod 16 mapped to hue; recently edited chunks appear as hot color | `chunk_version` |
| 4 (meshlets) | Each meshlet in a different hue band; reveals cluster quality and boundary errors | `meshlet_desc_pool`, `meshlet_range_table` |

Wireframe chunk AABB and meshlet AABB overlays are separate additive passes
(draw instanced quads from `chunk_aabb` / `meshlet_desc_pool.aabb`), toggled
independently of the fragment mode.

### Diagnostic Counters

A small storage buffer reset at frame start, atomically incremented by passes.
Read back alongside `queue_counts` (already accepted in the Stage 2 sync protocol).

```wgsl
struct DiagCounters {
    meshlets_culled:      atomic<u32>,
    chunks_empty_skipped: atomic<u32>,
    version_mismatches:   atomic<u32>,
    summary_rebuilds:     atomic<u32>,
    mesh_rebuilds:        atomic<u32>,
    cascade_ray_hits:     atomic<u32>,
    _pad:                 array<u32, 2>,
}

@group(0) @binding(N) var<storage, read_write> diag: DiagCounters;
```

Reset via `writeBuffer(diagBuffer, 0, new Uint32Array(8))` at frame start.
Read back with the same async pattern as `queue_counts`. Display in the existing
`debugOverlay.ts` HUD alongside chunk count and memory stats.

**Counters to instrument:**

| Counter | Set in | Detects |
|---|---|---|
| `meshlets_culled` | R-4 phase 2 per-meshlet cull | Culling effectiveness; should be high for typical scenes |
| `chunks_empty_skipped` | R-4 phase 1 `is_empty` check | Whether `chunk_flags.is_empty` is correctly populated |
| `version_mismatches` | Swap pass | Stale async results arriving after eviction; should be near zero |
| `summary_rebuilds` | I-3 pass | Rate of summary invalidation; spike on material changes is expected |
| `mesh_rebuilds` | R-1 pass | Rate of mesh invalidation; spike on edits is expected |
| `cascade_ray_hits` | R-6 per-hit | Approximate probe ray hit density; useful for cascade interval tuning |

### Spot-Reading Any Buffer

Between frames, any storage buffer can be mapped for one-off inspection:

```typescript
// CPU-side spot-read of queue_counts (or any storage buffer)
const readback = device.createBuffer({
    size: srcBuffer.size,
    usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ,
});
const encoder = device.createCommandEncoder();
encoder.copyBufferToBuffer(srcBuffer, 0, readback, 0, srcBuffer.size);
device.queue.submit([encoder.finish()]);
await readback.mapAsync(GPUMapMode.READ);
const data = new Uint32Array(readback.getMappedRange());
console.log(data);
readback.unmap();
readback.destroy();
```

This is a debugging tool, not a production path. It stalls pipeline execution and
must not appear in hot paths.

---

## Testing Strategy

The architecture has no `cargo test` path to WGSL shaders. The test strategy
compensates by layering tests at the boundaries where GPU logic can be reached.

### Test Tier Overview

| Tier | Tool | What it tests | Where |
|---|---|---|---|
| 1 | `wgpu` native Rust tests | Shader correctness — dispatch real WGSL, read back results | `crates/gpu_tests/` |
| 2 | CPU reference implementations | Algorithm correctness in pure Rust | alongside each algorithm |
| 3 | Protocol invariant tests | Edit protocol, version tagging, slot lifecycle — pure Rust | `crates/chunk_pool/tests/` |
| 4 | `wasm-pack test --headless` | Full WASM+WebGPU stack integration | `crates/wasm_*/` |
| 5 | Playwright (existing) | Visual regression | `apps/web/tests/` |

---

### Tier 1 — wgpu Native Shader Tests

`wgpu` runs natively (Vulkan / Metal / DX12) without a browser. A dedicated
`crates/gpu_tests/` crate (not compiled for WASM) can dispatch compute shaders
and assert on readback.

**Crate setup:**

```toml
# crates/gpu_tests/Cargo.toml
[dev-dependencies]
wgpu = { version = "22", features = ["vulkan", "metal", "dx12"] }
pollster = "0.4"   # for blocking on async in tests
```

**Test pattern:**

```rust
#[test]
fn summary_rebuild_empty_chunk_sets_is_empty() {
    pollster::block_on(async {
        let instance = wgpu::Instance::default();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default(), None).await.unwrap();

        // Upload known occupancy: all zeros (empty chunk)
        let occupancy_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[0u32; 2048]),
            usage: wgpu::BufferUsages::STORAGE,
        });

        // Output buffer for chunk_flags
        let flags_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        // Compile and dispatch the I-3 summary rebuild shader
        let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/summary_rebuild.wgsl"));
        // ... bind groups, pipeline, dispatch ...

        // Read back flags_buf
        let readback = /* ... */;
        let flags = /* ... */ as u32;
        assert_eq!(flags & IS_EMPTY_BIT, IS_EMPTY_BIT);
    });
}
```

**Priority tests for Tier 1:**

| Test | Shader | What to assert |
|---|---|---|
| Empty chunk → `is_empty = 1` | I-3 summary rebuild | `chunk_flags[slot] & IS_EMPTY_BIT` |
| Known occupancy → correct `occupancy_summary` | I-3 summary rebuild | All 16 u32 words match expected |
| Emissive palette entry → `has_emissive = 1` | I-3 emissive scan | `chunk_flags[slot] & HAS_EMISSIVE_BIT` |
| Stale bit set → queue entry appears | Compaction pass | `summary_rebuild_queue[0] == slot` |
| Version mismatch → no write | Swap pass | Output buffer unchanged |
| DDA hit at known position | R-6 traversal kernel | Hit coord matches expected |

---

### Tier 2 — CPU Reference Implementations

The greedy mesher already follows this pattern: the Rust implementation *is*
the reference, tested with `cargo test`. Extend it to the other algorithm-heavy
components.

**Components to reference-implement in Rust:**

| Algorithm | Reference location | Tests to write |
|---|---|---|
| DDA traversal (three-level) | `crates/voxelizer/src/dda.rs` | Hit detection for all 6 face normals; empty-chunk early exit; sub-brick skip |
| Summary rebuild | `crates/greedy_mesher/src/summary.rs` | OR-reduction over known `opaque_mask`; bricklet boundary alignment |
| Compaction pass logic | `crates/chunk_pool/src/compaction.rs` | Bitset scan + queue append matches expected output |
| `mat_is_emissive` | `crates/chunk_pool/src/material.rs` | All four f16 packing edge cases (zero, subnormal, max) |

The Rust reference and the WGSL shader are not tested against each other
automatically. The reference tests establish algorithmic correctness; if a
Tier 1 test fails, the reference implementation is the starting point for
diagnosis.

---

### Tier 3 — Protocol Invariant Tests

Most of the architectural correctness is in the protocols, not the math.
Protocol invariants can be tested in pure Rust with no GPU.

**Invariants to encode as tests:**

```rust
// Slot lifecycle invariant
#[test]
fn slot_is_resident_cleared_on_evict() {
    let mut pool = ChunkPool::new(64);
    let slot = pool.allocate(ChunkCoord::new(0, 0, 0));
    assert_eq!(pool.is_resident(slot), true);
    pool.evict(slot);
    assert_eq!(pool.is_resident(slot), false);
    assert!(pool.free_slots.contains(&slot));
}

// Version tagging invariant
#[test]
fn stale_rebuild_result_is_discarded() {
    let mut pool = ChunkPool::new(64);
    let slot = pool.allocate(ChunkCoord::new(0, 0, 0));
    let captured_version = pool.chunk_version(slot);

    // Simulate a voxel edit that increments version
    pool.increment_version(slot);

    // A rebuild that captured the old version should be rejected
    let accepted = pool.try_swap_mesh(slot, captured_version, dummy_mesh());
    assert!(!accepted);
}

// Queue population invariant
#[test]
fn stale_summary_bit_produces_queue_entry() {
    let mut control = EditControlPlane::new(64);
    control.set_stale_summary(7);   // slot 7

    let queue = control.run_compaction();
    assert!(queue.contains(&7u32));
    assert_eq!(control.stale_summary(7), false);   // bit cleared after compaction
}

// Edit protocol: CPU must not write queues directly
#[test]
#[should_panic]
fn cpu_cannot_write_rebuild_queue_directly() {
    let mut control = EditControlPlane::new(64);
    control.push_summary_rebuild_queue(7);   // must panic
}
```

These tests are fast (no GPU, no async), run in CI, and catch the design-level
bugs that Tier 1 tests cannot easily express.

---

### Tier 4 — wasm-pack Headless Browser Tests

For correctness checks that require the full WASM + WebGPU stack:

```bash
wasm-pack test crates/wasm_voxelizer --headless --chrome
```

These tests run in a real headless Chrome with GPU access. Use them for
end-to-end pipeline correctness that cannot be captured at a lower tier:

- Voxelization → occupancy upload → summary rebuild → `is_empty` correct
- Edit round-trip: CPU edit → dirty bit → GPU propagation → mesh rebuild → visual output
- Material system: register material → voxelize → `has_emissive` set correctly

Headless browser tests are slower (5–30s per test) and require a real GPU in
the CI environment. Run them as a pre-merge check, not on every commit.

### Tier 5 — Playwright Visual Regression (Existing)

The existing tests in `apps/web/tests/basic.spec.ts` cover gross visual
correctness (page load, triangle render, screenshot regression). These remain
the last line of defense for the full render stack, but they cannot diagnose
protocol or shader bugs — they can only detect that something went wrong.

Add scene-level regression tests as major systems are implemented:
- Voxelized OBJ loads and renders without artifacts
- Emissive chunks illuminate surrounding surfaces (cascade GI)
- Edit operation produces correct mesh update

---

## What Needs to Be Built

| Component | Status | Blocks |
|---|---|---|
| Timestamp query setup + `GPUQuerySet` | Not implemented | Pass timeline |
| Readback buffer + async read pipeline | Not implemented | Pass timeline |
| Pass timeline canvas overlay | Not implemented | Profiling |
| `debug_mode` uniform + fragment shader modes | Not implemented | State visibility |
| AABB wireframe overlay pass | Not implemented | Chunk/meshlet debug |
| `DiagCounters` buffer + per-pass atomics | Not implemented | Diagnostic HUD |
| `crates/gpu_tests/` Tier 1 test crate | Not implemented | Shader correctness |
| CPU reference: DDA, summary rebuild, compaction | Not implemented | Algorithm regression |
| Protocol invariant tests | Not implemented | Edit protocol correctness |

Build order: diagnostic counters + HUD readback (quick win, extends existing
`debugOverlay.ts`) → debug render modes → timestamp query + timeline → Tier 1
shader tests → Tier 3 protocol tests.

---

## See Also

- [pipeline-stages](pipeline-stages.md) — pass sequence and buffer ownership; one timestamp pair per stage
- [edit-protocol](edit-protocol.md) — protocol invariants tested in Tier 3; `queue_counts` readback timing
- [gpu-chunk-pool](gpu-chunk-pool.md) — slot lifecycle invariants tested in Tier 3
- [extension-seams](extension-seams.md) — architectural integration test concept; Tier 3 tests formalize this
