# Viewport Architecture

**Type:** spec
**Status:** current
**Date:** 2026-03-24
**Depends on:** ADR-0014

> How the GPU rendering surface integrates with the Phi panel system. The viewport is not a Svelte component — it is a raw canvas element managed outside the reactive framework.

---

## Principle

Professional CG applications (Three.js editor, Godot, Blender) treat the GPU rendering surface as a **raw element** that the panel/layout system provides a container for. The renderer appends its own canvas as a child of the container. The layout system controls the container's size and position; the renderer controls the canvas's content.

Phi follows this pattern. The viewport is a dock panel type that owns a container div. The renderer creates a canvas, appends it to the container, acquires a WebGPU context, and drives the render loop. Svelte's reactive DOM does not manage the canvas element.

---

## Architecture

```
┌─ Phi DockLayout ──────────────────────────────────────────────┐
│                                                                │
│  ┌─ Viewport Panel (container div) ────────────────────────┐  │
│  │                                                          │  │
│  │  ┌─ HTMLCanvasElement (raw, not Svelte-managed) ──────┐  │  │
│  │  │                                                     │  │  │
│  │  │   GPUCanvasContext                                  │  │  │
│  │  │   → getCurrentTexture() per frame                   │  │  │
│  │  │   → render passes write to surface texture          │  │  │
│  │  │   → compositor presents at next vsync               │  │  │
│  │  │                                                     │  │  │
│  │  └─────────────────────────────────────────────────────┘  │  │
│  │                                                          │  │
│  │  ResizeObserver → canvas.width/height → surface.configure │  │
│  │  Pointer events → orbit camera → camera uniform update    │  │
│  │                                                          │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                │
│  ┌─ Inspector Panel ──┐  ┌─ Performance Panel ──┐             │
│  │  Svelte components  │  │  Svelte components   │             │
│  └────────────────────┘  └──────────────────────┘             │
└────────────────────────────────────────────────────────────────┘
```

---

## Canvas Lifecycle

### Creation

1. Phi DockLayout mounts the viewport panel container (a `<div>`)
2. Application code creates a `<canvas>` element via `document.createElement("canvas")`
3. Canvas is appended to the container: `container.appendChild(canvas)`
4. WebGPU context is acquired: `canvas.getContext("webgpu")`
5. wgpu surface is created from the canvas (main thread, not transferred to worker)

### Sizing

1. ResizeObserver watches the container div
2. On resize: read `container.clientWidth` / `container.clientHeight`
3. Set `canvas.width = clientWidth * devicePixelRatio`, same for height
4. Reconfigure the wgpu surface with the new dimensions
5. Recreate depth texture at the new size

### Input

1. Pointer events (`pointerdown`, `pointermove`, `pointerup`, `wheel`) are attached to the canvas element directly
2. Event handlers compute orbit camera parameters (yaw, pitch, distance)
3. Camera position/direction are passed to the Renderer's `set_camera()` method
4. Camera uniform buffer is updated at the start of each frame

### Destruction

1. When the dock panel is removed (not hidden — Phi's `always` render mode keeps hidden panels alive):
2. Cancel the `requestAnimationFrame` loop
3. Drop the Renderer (releases GPUDevice, buffers, pipelines)
4. Remove the canvas from the DOM

---

## Render Loop

Driven by `requestAnimationFrame` on the **main thread**:

```
rAF callback:
  1. Check pending data from worker (posted ArrayBuffers)
  2. Upload new data to GPU if any
  3. Update camera uniform
  4. Dispatch compute passes if dirty (I-3, R-1, build_indirect, build_wireframe)
  5. Dispatch render passes (R-2 depth, R-5/R-9 color)
  6. queue.submit()
  7. [implicit: compositor presents at next vsync]
  8. Post frame timing to stores
  9. Schedule next rAF
```

The render loop does NOT call `present()` explicitly — wgpu's WebGPU backend handles presentation automatically when the next `getCurrentTexture()` is called. The compositor displays the completed frame at the next vsync because the main thread's rAF is synchronized with the compositor.

---

## Phi Integration

### What Phi provides

- **Container div**: a dock panel with known size, position, and visibility state
- **Panel lifecycle events**: `onDidVisibilityChange` for pausing the render loop when the panel is hidden
- **Layout constraints**: `minimumWidth`, `minimumHeight` respected by the split/grid layout
- **Tab system**: viewport can be tabbed alongside other panels if needed

### What Phi does NOT provide (application responsibility)

- Canvas creation and GPU context acquisition
- Render loop management
- Input handling for camera controls
- Resize propagation to the GPU surface

### Future: Phi Viewport Primitive

As the pattern matures, Phi may provide a first-class `Viewport` component that encapsulates:
- Raw canvas creation + append
- ResizeObserver setup
- Pointer event routing with configurable handlers
- Render loop lifecycle (start/stop/pause on visibility change)

This would be a non-reactive, imperative component — unlike Section, PropRow, etc. which are reactive Svelte components. The distinction matters: the viewport's canvas is a raw DOM element that Svelte should not reconcile or diff.

---

## Differences from ADR-0013 Architecture

| Aspect | ADR-0013 (superseded) | ADR-0014 (current) |
|---|---|---|
| Canvas type | OffscreenCanvas (transferred to worker) | HTMLCanvasElement (main thread) |
| GPU device location | Worker thread | Main thread |
| Render loop driver | Worker's `requestAnimationFrame` | Main thread's `requestAnimationFrame` |
| Frame presentation | Compositor-decoupled (tearing) | Compositor-synced (no tearing) |
| Camera commands | Binary protocol → worker → WASM | Direct WASM call on main thread |
| Resize flow | ResizeObserver → protocol → worker → WASM | ResizeObserver → direct WASM call |

---

## See Also

- [ADR-0014](../adr/0014-main-thread-gpu-worker-cpu.md) — architectural decision
- [thread-boundary](thread-boundary.md) — main thread vs worker responsibilities
- [pipeline-stages](pipeline-stages.md) — stage execution order
