# UI Migration Plan

Transition from the current vanilla-TS DOM testbed to the Svelte 5 + Tailwind 4
design system, rebuilt around the GPU-native architecture's actual surface area.

Related: [[ui-design-system]] (component and token reference), [[debug-profiling]]
(GPU panel data sources), [[gpu-chunk-pool]] (pool panel data), [[edit-protocol]]
(protocol panel data).

---

## Why Now

The current UI was designed for the CPU ChunkManager model. It exposes voxelizer
settings and mesh toggles because those were the control flow knobs. The GPU-native
architecture has a different internal surface — chunk pool slots, edit protocol
queues, pass timings, debug render modes — and the current UI has no place to
put any of it.

The rebuild is forced by the architecture change regardless of the framework
choice. Doing both at once is correct.

---

## Current State

```
apps/web/src/
├── main.ts              250 lines — entry point, imperative DOM construction
├── style.css            285 lines — all styling, no framework
└── ui/
    ├── animationLoop.ts  97 lines — frame rate limiting, FPS tracking
    ├── debugOverlay.ts  216 lines — floating HUD (memory, perf, chunks)
    ├── debugPanel.ts    257 lines — controls sidebar (toggles, sliders, selects)
    ├── scenePanel.ts     37 lines — wireframe toggle, frame object
    └── uiApi.ts         168 lines — module builder contract (addSlider, addButton…)
```

Technology: vanilla TypeScript, `document.createElement()` throughout, no
framework, no CSS utility library, no component library.

The rendering layer (`viewer/`, `modules/`, WASM crates) does not change.

---

## Target State

### Tech Stack

| Layer | Package | Notes |
|---|---|---|
| Framework | `svelte@5` | Runes-based reactivity |
| Build | `@sveltejs/vite-plugin-svelte` | Vite plugin, no SvelteKit |
| CSS | `tailwindcss@4` + `@tailwindcss/vite` | Vite plugin, no postcss |
| Headless | `bits-ui@2` | Accessible behavior layer |
| Variants | `tailwind-variants` | `tv()` for component variants |
| Merge | `clsx` + `tailwind-merge` | `cn()` helper |
| Icons | `lucide-svelte` | Consistent icon set |
| Fonts | Geist + Geist Mono (variable WOFF2) | Self-hosted, `font-display: swap` |

### Panel Architecture

The sidebar is a vertical `<Tabs.Root orientation="vertical">` from Bits UI.
Visually it is a 56px icon column; behaviorally it is a tab group —
one active panel at a time, arrow-key navigation, correct `aria-selected`
and focus management provided by the primitive.

The horizontal tab strip in [[ui-design-system]] Section 5 is a separate
component used for sub-views *within* a panel (e.g., Timeline / Counters
inside PerformancePanel). The sidebar is the outer navigation layer.

```
Tabs.Root orientation="vertical"   ← the entire app shell
├── Tabs.List                       56px icon column (the sidebar)
│   ├── Tabs.Trigger value="scene"  LayersIcon
│   ├── Tabs.Trigger value="pool"   DatabaseIcon
│   ├── Tabs.Trigger value="proto"  ActivityIcon
│   ├── Tabs.Trigger value="perf"   BarChart2Icon
│   ├── Tabs.Trigger value="debug"  BugIcon
│   └── Tabs.Trigger value="settings" SettingsIcon  ← pinned to bottom
├── Tabs.Content value="scene"      → ScenePanel
├── Tabs.Content value="pool"       → GpuPoolPanel
├── Tabs.Content value="proto"      → EditProtocolPanel
├── Tabs.Content value="perf"       → PerformancePanel
├── Tabs.Content value="debug"      → DebugPanel
├── Tabs.Content value="settings"   → SettingsPanel
└── Viewport.svelte                 ← outside tab content, always rendered
    └── StatsBar.svelte               on-canvas FPS / frame ms HUD
```

`Viewport.svelte` lives alongside the `Tabs.Content` elements, not inside
any of them — the canvas must always be mounted and receiving the render loop
regardless of which panel is active.

### Panel Definitions

#### ScenePanel
*Replaces: `scenePanel.ts` + model loading controls from `debugPanel.ts`*

- OBJ file picker (file input)
- Loaded model list with name, triangle count, status badge
- Voxelization parameters: grid dim, voxel size (from current module controls)
- Load / Re-voxelize button
- Wireframe toggle
- Frame Object button

#### GpuPoolPanel
*New — no current equivalent*

- Slot utilization: `N used / N_SLOTS` with a fill bar
- Memory breakdown table: occupancy atlas, palette + index, summaries, mesh pool
  (values from [[gpu-chunk-pool]] memory budget; sourced from CPU-side accounting)
- Resident chunk list: scrollable table of active slots with coord, version,
  `is_empty` / `has_emissive` flags
- Evict button per slot (dev tool)

#### EditProtocolPanel
*New — no current equivalent*

- Queue depths (from `queue_counts` readback): mesh rebuild, summary rebuild,
  lighting — each shown as a count with a small history sparkline
- Active dirty chunk count (from `stale_summary` popcount readback)
- Version mismatch counter (from `DiagCounters.version_mismatches`)
- Pass last run timestamps (when did each queue last flush)

Data source: the same `queue_counts` + `DiagCounters` readback already
specified in [[debug-profiling]] — one async readback per frame, displayed here.

#### PerformancePanel
*New — no current equivalent; replaces floating FPS counter*

- Pass timeline canvas (scrolling stacked bar chart, per [[debug-profiling]])
- Per-pass duration table: current frame, min, max, avg
- Frame budget indicator (16.67ms / 33.33ms lines)
- Timestamp query availability badge (enabled / fallback)

#### DebugPanel
*Replaces: most of `debugPanel.ts`*

- Debug render mode selector (radio group): Normal / Bricklet Occupancy /
  Emissive / Version Heatmap / Meshlet Clusters
- Overlay toggles: Chunk AABBs, Meshlet AABBs, `chunk_version` labels
- Cascade debug: show probe positions, show individual cascade layers
- GPU diagnostic counters display (from `DiagCounters` readback)

#### SettingsPanel
*Replaces: renderer/frame rate/resolution controls from `debugPanel.ts`*

- Renderer preference (WebGPU / WebGL2)
- Target frame rate (0 = uncapped, 30, 60, 120)
- Resolution lock (width × height input)
- WebGPU error log (textarea, toggle)

---

## The Module API Bridge

The critical design constraint: rendering modules (TypeScript, WASM) call `uiApi`
imperatively from outside any Svelte component. They must continue to work
without change.

Solution: `uiApi.ts` becomes a **Svelte writable store**. Modules call the same
functions as before; the module panel reads the store reactively.

```typescript
// apps/web/src/ui/uiApi.ts (new)
import { writable } from "svelte/store";

export type Control =
    | { kind: "slider";   id: string; label: string; value: number; min: number; max: number; step?: number; onChange: (v: number) => void }
    | { kind: "number";   id: string; label: string; value: number; onChange: (v: number) => void }
    | { kind: "checkbox"; id: string; label: string; checked: boolean; onChange: (v: boolean) => void }
    | { kind: "select";   id: string; label: string; value: string; options: string[]; onChange: (v: string) => void }
    | { kind: "button";   id: string; label: string; onClick: () => void }
    | { kind: "text";     id: string; label: string; value: string }
    | { kind: "file";     id: string; label: string; accept: string; onChange: (f: File) => void };

export const moduleControls = writable<Control[]>([]);

// Public API — identical call signature to current uiApi.ts
export function addSlider(id, label, value, min, max, onChange, step?) {
    moduleControls.update(cs => [...cs, { kind: "slider", id, label, value, min, max, step, onChange }]);
}
export function addButton(id, label, onClick) {
    moduleControls.update(cs => [...cs, { kind: "button", id, label, onClick }]);
}
// … etc for all current control types

export function clearModuleControls() {
    moduleControls.set([]);
}
```

`ModulePanel.svelte` reads `$moduleControls` and renders whatever is in it.
No existing module code changes. The store is the only new concept.

The one breaking change: `uiApi.ts` currently returns a builder object per
module section. The new API is flat (one global list). If section grouping is
needed, add a `{ kind: "section-header"; label: string }` control type.

---

## File Migration Map

| Current | New | Notes |
|---|---|---|
| `index.html` | `index.html` | Mount point only; `.dark` class on root |
| `style.css` | `app.css` | OKLCH tokens, Tailwind directives, layout shell |
| `main.ts` | `main.ts` | Mount `App.svelte`; pass canvas ref to viewer |
| `ui/animationLoop.ts` | `ui/animationLoop.ts` | No change; feeds `PerformancePanel` store |
| `ui/debugOverlay.ts` | `StatsBar.svelte` | Minimal on-canvas HUD, or eliminate entirely |
| `ui/debugPanel.ts` | `DebugPanel.svelte` + `SettingsPanel.svelte` | Split by concern |
| `ui/scenePanel.ts` | `ScenePanel.svelte` | Extended with model list and voxelizer params |
| `ui/uiApi.ts` | `ui/uiApi.ts` (store-backed) | Same external API, writable store backing |
| *(none)* | `GpuPoolPanel.svelte` | New |
| *(none)* | `EditProtocolPanel.svelte` | New |
| *(none)* | `PerformancePanel.svelte` | New (houses pass timeline canvas) |

---

## Dependency Changes

```jsonc
// apps/web/package.json — additions
{
  "dependencies": {
    "bits-ui": "^2.0.0",
    "clsx": "^2.0.0",
    "lucide-svelte": "^0.577.0",
    "tailwind-merge": "^2.0.0",
    "tailwind-variants": "^0.3.0"
  },
  "devDependencies": {
    "@sveltejs/vite-plugin-svelte": "^5.0.0",
    "@tailwindcss/vite": "^4.0.0",
    "svelte": "^5.0.0",
    "tailwindcss": "^4.0.0"
  }
}
```

```typescript
// apps/web/vite.config.ts — add two plugins
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";

// In plugins array, before existing wasm/alias plugins:
svelte(),
tailwindcss(),
```

Font files: `GeistVariableVF.woff2` and `GeistMonoVariableVF.woff2` go in
`apps/web/public/assets/fonts/`. Source from `vercel/geist-font` releases.

---

## Build Phases

### Phase 1 — Foundation (no visible change)

Install deps, update vite config, create `app.css` with OKLCH tokens and
Tailwind directives, add font files, create `App.svelte` that mounts and renders
a `<canvas>` only. The canvas is passed to the existing viewer. Everything else
stays as-is. Goal: working build with Svelte and Tailwind loaded.

Deliverable: `pnpm dev` works, page renders identically to today, Svelte and
Tailwind are loaded and active.

### Phase 2 — Shell

Implement `Sidebar.svelte` (icon nav, status indicator, tooltips), `PanelArea.svelte`
(active panel slot), `Viewport.svelte` (canvas mount). Add stub panel components
that display placeholder content. This is the first visual change.

Deliverable: sidebar nav is visible and functional; clicking icons switches the
active panel (content is placeholder).

### Phase 3 — Port Existing Panels

Implement `ScenePanel.svelte`, `DebugPanel.svelte`, and `SettingsPanel.svelte`
from the existing `scenePanel.ts`, `debugPanel.ts` content. Port `uiApi.ts` to
the store backing and implement `ModulePanel.svelte`. At the end of this phase,
all current functionality exists in the new UI.

Deliverable: feature parity with current UI in the new framework. Existing
Playwright tests pass.

### Phase 4 — GPU-Native Panels

Implement `GpuPoolPanel.svelte`, `EditProtocolPanel.svelte`, and
`PerformancePanel.svelte`. These require data flowing from the GPU runtime —
stub them with mock data initially, wire to real readbacks as the GPU-native
arch is implemented.

Deliverable: panels exist and display data (mock or real depending on
implementation state of GPU arch).

### Phase 5 — StatsBar + Polish

Replace `debugOverlay.ts` HUD with `StatsBar.svelte` (minimal, on-canvas FPS
and frame ms). Apply full Viaduct design system polish per [[ui-design-system]].
Remove `style.css`.

Deliverable: `style.css` deleted; all styling from `app.css` + Tailwind classes.

---

## What Does Not Change

- `viewer/` and all rendering code — Three.js, WebGPU pipeline, WASM modules
- `modules/` — voxelizer, OBJ loader; their `uiApi` calls are unchanged
- `animationLoop.ts` — drives the render loop, frame limiting, FPS calculation
- `playwright` tests — they test canvas output, not DOM structure
- The `uiApi` external call signature — modules call the same functions

---

## Risk Surface

**`uiApi` store transition** — the only genuine design risk. The store-backed API
must maintain the full current call surface and handle modules that call `uiApi`
before the Svelte app is mounted (module init may run early). Mitigation: the
store is initialized before the Svelte app mounts; writes to it before mount are
buffered and rendered on first mount.

**Canvas ownership** — Three.js needs the canvas element reference. In Svelte,
`bind:this={canvas}` inside `onMount` provides it after the DOM is ready.
The viewer's `attachCanvas(canvas)` call moves from `main.ts` into the
`Viewport.svelte` `onMount` callback.

**GPU panel data without GPU arch** — `GpuPoolPanel` and `EditProtocolPanel`
have no real data until the GPU-native runtime is implemented. Stub with mock
data (`$gpuPool = { slotsUsed: 0, slotsTotal: 1024 }`) during Phase 4 so the
panels can be built and styled independently.

---

## See Also

- [[ui-design-system]] — component reference, token values, glassmorphism patterns
- [[debug-profiling]] — `DiagCounters` and `queue_counts` data sources for EditProtocol and Performance panels
- [[gpu-chunk-pool]] — memory budget figures for GpuPool panel
- [[edit-protocol]] — queue semantics displayed in EditProtocol panel
