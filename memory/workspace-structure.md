---
name: Workspace Structure
description: Current accurate monorepo layout after 2026-03-21 reorganization — replaces stale MEMORY.md architecture section
type: project
---

## Monorepo Layout (as of 2026-03-21)

```
Gestalt/
├── apps/
│   └── gestalt/              ← primary app (was apps/testbed)
│       ├── src/
│       │   ├── App.svelte    ← shell mount + renderer bridge init
│       │   ├── renderer/     ← NEW: renderer worker + protocol
│       │   │   ├── protocol.ts           — binary command types + SAB layout
│       │   │   ├── renderer.worker.ts    — worker entry (stub)
│       │   │   └── RendererBridge.ts     — main-thread bridge
│       │   └── lib/
│       │       ├── components/
│       │       │   ├── panels/   — DebugPanel, PerformancePanel, ScenePanel, etc.
│       │       │   ├── shell/    — Sidebar, PanelArea, Viewport, StatusBar
│       │       │   └── viz/      — TimelineCanvas, PassBreakdownTable (store-coupled)
│       │       ├── stores/       — viewer, timeline, status, rendererBridge
│       │       └── utils/        — gpu.ts
│       ├── tests/            ← Playwright E2E tests
│       ├── playwright.config.ts
│       ├── svelte.config.js
│       └── vite.config.ts    ← COOP/COEP headers, coi plugin, WASM manualChunks
├── packages/
│   ├── phi/                  ← @gestalt/phi: 12 primitive UI components
│   │   └── src/              — vitest unit tests, test-setup.ts
│   └── voxelizer-js/         ← TypeScript wrapper for WASM voxelizer
├── crates/                   ← Rust crates (see crate-deprecation-plan.md)
│   ├── greedy_mesher/        — binary greedy meshing (64³ chunks, ~8.5K lines)
│   ├── voxelizer/            — GPU mesh-to-voxel rasterization (~3.2K lines)
│   ├── wasm_greedy_mesher/   — WASM bindings (wasm-bindgen-test tests added)
│   ├── wasm_voxelizer/       — WASM bindings
│   ├── wasm_obj_loader/      — OBJ parser WASM bindings
│   └── wasm_webgpu_demo/     — WebGPU demo WASM bindings
├── legacy/
│   ├── apps/web/             ← was apps/web (Three.js + module system)
│   └── packages/modules/     ← was packages/modules (@gestalt/modules)
├── docs/
│   ├── adr/                  ← all ADRs 0001–0012
│   ├── architecture/
│   │   └── wasm-boundary-protocol.md
│   ├── README.md
│   └── vault-guide.md
└── memory/                   ← persistent Claude memory files
```

## Build & Dev

```bash
pnpm dev              # starts apps/gestalt at http://localhost:5173
pnpm dev:legacy       # starts legacy/apps/web
pnpm build:wasm       # compile all Rust crates → apps/gestalt/src/wasm/
pnpm build            # Vite production build → apps/gestalt/dist/
```

## Key Config Files

| File | Purpose |
|---|---|
| `pnpm-workspace.yaml` | includes apps/*, packages/*, crates/*, legacy/apps/*, legacy/packages/* |
| `apps/gestalt/vite.config.ts` | COOP/COEP headers, coi-serviceworker plugin, WASM chunk splitting, @web transitional alias |
| `apps/gestalt/svelte.config.js` | vitePreprocess for svelte-check |
| `apps/gestalt/index.html` | coi-serviceworker script (before main.ts) |

## Documentation Structure

- `docs/adr/` — all 12 ADRs consolidated here
- `docs/architecture/wasm-boundary-protocol.md` — binary protocol spec (partially implemented)
- `docs/vault-guide.md` — what to copy to `../Gestalt-vault/` (Obsidian)
- `docs/README.md` — index

## Active @web Dependency (transitional)

`apps/gestalt/src/lib/components/shell/Viewport.svelte` and `stores/viewer.ts` import from `@web/*` (the Three.js backend in `legacy/apps/web/`). This alias exists in `vite.config.ts` marked as transitional. It will be removed when the renderer worker owns the frame loop (ADR-0011 Phase 3+).
