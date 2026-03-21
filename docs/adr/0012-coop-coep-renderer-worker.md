# ADR-0012 — Cross-Origin Isolation and Renderer Worker Architecture

**Type:** adr
**Status:** accepted
**Date:** 2026-03-21

---

## Context

The application requires `SharedArrayBuffer` for the high-frequency state readback path described in the WASM boundary protocol (see `docs/architecture/wasm-boundary-protocol.md`). `SharedArrayBuffer` was re-enabled in browsers in 2020 behind mandatory cross-origin isolation headers:

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

These headers cannot be set later without coordination across the entire deployment stack. Deferring them to a later sprint would require rework of the deployment pipeline, service worker strategy, and potentially break third-party integrations. They must be set from the start.

Additionally, the renderer worker entry point and binary protocol types must be established before any WASM integration work begins. The protocol is a typed binary contract; retrofitting it onto existing imperative code is significantly more disruptive than defining it upfront.

---

## Decision

### 1. COOP/COEP headers — dual-mode strategy

**Dev server:** `vite.config.ts` sets `server.headers` and `preview.headers` with COOP/COEP directly. No service worker overhead in development.

**Production (GitHub Pages):** GitHub Pages does not support custom HTTP response headers. The industry-standard solution is `coi-serviceworker` (MIT, ~60 lines, no runtime dependencies). The service worker intercepts the initial page load, reloads the page if the headers are not present, and caches the cross-origin-isolated context for subsequent navigations.

The `coi-serviceworker.js` file is:
- Installed as a `devDependency` (`coi-serviceworker@^0.1.7`)
- Emitted to the `dist/` root as an asset by a Vite plugin in `vite.config.ts`
- Registered by a `<script src="/coi-serviceworker.js">` tag in `index.html` that executes before the application module

This means `crossOriginIsolated === true` and `typeof SharedArrayBuffer !== "undefined"` in all environments once deployed.

### 2. Renderer worker — stub now, wired later

The renderer worker entry point (`src/renderer/renderer.worker.ts`) is created at this milestone as a typed stub. It:
- Allocates the `SharedArrayBuffer` ring buffer and snapshot buffer
- Posts a `"ready"` message to the main thread with the SAB handles
- Decodes incoming command buffers per the binary protocol
- Dispatches to stub handlers that will be filled in when WASM is wired

The worker is created by `RendererBridge.create()` (an async factory on the main thread) which waits for the `"ready"` message before resolving. The bridge is stored in `rendererBridgeStore` (a Svelte writable), initialized in `App.svelte`.

The worker does NOT currently own a WebGPU device or run a frame loop. That integration happens when the WASM renderer is ready.

### 3. Binary command protocol — established now

`src/renderer/protocol.ts` defines the permanent typed contract:
- Five opcodes with fixed-size payloads (no length prefixes, no JSON)
- Ring buffer layout constants (matches `HISTORY_FRAMES` in `stores/timeline.ts`)
- Snapshot buffer layout constants
- TypeScript types for all worker/main-thread message shapes

This file is the single source of truth for the boundary. Both `renderer.worker.ts` and `RendererBridge.ts` import from it. Any protocol change requires updating this file and both sides simultaneously — intentional, to prevent drift.

### 4. WASM chunk splitting

`vite.config.ts` `build.rollupOptions.output.manualChunks` splits each WASM package glue module into its own output chunk:
- `wasm-obj-loader`
- `wasm-voxelizer`
- `wasm-greedy-mesher`
- `wasm-webgpu-demo`

This allows the browser to cache each WASM module independently and load only the modules required for the active scene.

### 5. `@web` alias — kept, marked transitional

`Viewport.svelte` and `stores/viewer.ts` currently import from `@web/*` (the Three.js backend in `legacy/apps/web/`). This alias is retained in `vite.config.ts` with an explicit comment marking it as transitional. It is scheduled for removal when the renderer worker owns the frame loop and the Three.js backend is no longer needed by the active app.

---

## Consequences

**Positive:**
- `SharedArrayBuffer` is guaranteed available in all environments (dev + GitHub Pages)
- The renderer worker architecture is established — WASM integration drops directly into `renderer.worker.ts` with no structural changes
- Binary protocol is typed and version-controlled from day one
- WASM chunks are cache-isolated, reducing initial load for scenes that don't need all modules
- `svelte-check` now resolves `.svelte` files correctly via `svelte.config.js`

**Negative / Trade-offs:**
- `coi-serviceworker` adds a service worker registration step; first-load UX has a brief reload if headers are not already set
- COOP/COEP blocks `document.domain` mutation and restricts `window.opener` access — acceptable for a first-party app with no third-party iframe embeds
- The `@web` alias creates a cross-package dependency on legacy code that must be cleaned up when the renderer worker takes over

---

## Files Changed

| File | Change |
|---|---|
| `apps/gestalt/package.json` | Added `coi-serviceworker` devDependency |
| `apps/gestalt/vite.config.ts` | COOP/COEP headers, coi plugin, WASM manualChunks, @web comment |
| `apps/gestalt/index.html` | coi-serviceworker script, title update |
| `apps/gestalt/svelte.config.js` | Created: vitePreprocess for svelte-check |
| `apps/gestalt/src/renderer/protocol.ts` | Created: binary command types, SAB layout constants |
| `apps/gestalt/src/renderer/renderer.worker.ts` | Created: worker entry, command decoder, SAB allocation |
| `apps/gestalt/src/renderer/RendererBridge.ts` | Created: main-thread bridge, typed command builders |
| `apps/gestalt/src/lib/stores/rendererBridge.ts` | Created: Svelte store holding the bridge instance |
| `apps/gestalt/src/App.svelte` | Bridge init in `$effect`, cleanup on destroy |
