# WASM Boundary Protocol

**Type:** spec
**Date:** 2026-03-21

> Status: **Partially implemented** — protocol types, worker entry point, and main-thread bridge are in place (ADR-0012). WASM and WebGPU are not yet wired into the worker.
>
> **Implementation files:**
> - `apps/gestalt/src/renderer/protocol.ts` — binary types, SAB layout constants
> - `apps/gestalt/src/renderer/renderer.worker.ts` — worker entry (stub; SAB allocated, command decoder live)
> - `apps/gestalt/src/renderer/RendererBridge.ts` — main-thread typed command builder + bridge lifecycle
> - `apps/gestalt/src/lib/stores/rendererBridge.ts` — Svelte store holding the active bridge

---

## Overview

The boundary follows a two-tier model. The JS layer never calls WASM functions directly on the main thread; all communication is typed binary.

```
Main thread (Svelte)              Renderer Worker
      │                                 │
      │── command queue ───────────────►│  (Transferable ArrayBuffer)
      │                                 │── executes frame ──────────►│
      │◄── state readback ──────────────│  (SharedArrayBuffer read)
      │                                 │
[Svelte stores update]
```

---

## Tier 1 — Command Queue (JS → WASM)

**Transport:** `Transferable ArrayBuffer` posted to the renderer worker.

**Format:** Packed sequence of typed command structs. Each command starts with a 1-byte opcode followed by fixed-size payload.

**JS never holds WASM object references.** The GUI holds handles — integer IDs or slot indices — and the command payload carries those handles. The WASM side maps handles to its own internal objects.

**Examples:**

| Opcode | Name | Payload |
|--------|------|---------|
| `0x01` | `LoadChunk` | `chunkId: u32, slotIndex: u16` |
| `0x02` | `UnloadChunk` | `chunkId: u32` |
| `0x03` | `SetCamera` | `pos: f32×3, dir: f32×3` |
| `0x04` | `SetRenderMode` | `mode: u8` |
| `0x05` | `ResizeViewport` | `width: u16, height: u16` |

---

## Tier 2 — State Readback (WASM → JS)

### 2a — High-Frequency: Ring Buffer

**Transport:** `SharedArrayBuffer` ring buffer.
**Frequency:** Written every frame by the renderer worker; read by the main thread on `requestAnimationFrame`.
**Contents:** Frame timings, GPU counters, pool occupancy.
**Cost:** Zero synchronization. Main thread reads stale-by-one-frame data, which is acceptable for diagnostic display.

**Layout (fixed, never grows):**

```
Offset  Size  Field
0       4     head (frame index, wraps at ring capacity)
4       4     capacity
8+      N×FRAME_STRIDE  ring entries
```

Each `FRAME_STRIDE` entry:

```
0   4   totalMs: f32
4   4   passCount: u32
8   N×8 passes: [{ nameHash: u32, ms: f32 }]
```

The Svelte `frameTimeline` store reads this on each rAF tick.

### 2b — Low-Frequency: Snapshot

**Transport:** Fixed-layout `SharedArrayBuffer` snapshot.
**Frequency:** Written by WASM on request (not every frame); read by JS with `DataView`.
**Contents:** Chunk list, scene object metadata, GPU pool state.
**Cost:** No allocation, no GC pressure. JS reads a predetermined fixed layout.

---

## SharedArrayBuffer Requirements

`SharedArrayBuffer` requires cross-origin isolation headers:

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

**Dev (Vite):** Use `vite-plugin-cross-origin-isolation` or set headers manually in `vite.config.ts`.

**GitHub Pages:** Add a `_headers` file at the repo root (Cloudflare Pages style) or use a service worker to inject headers.

---

## Svelte Store Mapping

| WASM output | Svelte store | Update path |
|-------------|-------------|-------------|
| Frame timings | `frameTimeline` | rAF ring buffer read |
| GPU counters | `diagCounters` | rAF ring buffer read |
| Diag history | `diagHistory` | rAF ring buffer read |
| Pool occupancy | `gpuPoolStore` | rAF ring buffer read |
| Scene objects | `sceneStore` | Low-freq snapshot |

---

## Key Rules

1. **Handles, not references** — GUI stores integer IDs. WASM owns the object graph.
2. **No WASM imports on main thread** — All WASM interaction goes through the renderer worker.
3. **Typed binary only** — No JSON, no JS object serialization at the boundary.
4. **Stores are the receiving end** — Svelte components read stores; they never read the SAB directly.
