# Gestalt Project Memory

## Project

- **Repo:** /Users/taylorhale/Documents/dev_hub/repos/Gestalt
- **Live demo:** https://anjin-byte.github.io/Gestalt/
- **Stack:** Rust/WASM + Svelte 5 + WebGPU, pnpm monorepo, Vite, wasm-pack
- **Goal:** GPU-driven voxel mesh renderer — heavyweight in-browser application

## Workspace & Structure

[`memory/workspace-structure.md`](./workspace-structure.md) — current monorepo layout, key config files, build commands, active @web alias (transitional). Reorganization completed 2026-03-21 (apps/testbed→apps/gestalt, legacy/, @gestalt/phi package).

## Architecture Decisions

**ADR index:** `docs/adr/` — ADRs 0001–0012 all consolidated here.

| ADR | Summary | Status |
|---|---|---|
| 0001 | Three.js renderer | Accepted, amended by 0011 |
| 0002 | Module contract | Accepted (module system now in legacy/) |
| 0003 | Binary greedy meshing | Accepted |
| 0004 | 64³ chunk size | Accepted |
| 0005 | Voxelizer integration | Superseded by 0009 |
| 0006 | LOD — point mode for distant | Proposed |
| 0007 | Material strategy — texture atlas | Proposed |
| 0008 | Design gap mitigations | Proposed |
| 0009 | GPU-compact Architecture B | Accepted |
| 0010 | Radiance cascades — hybrid screenspace + world-space | Proposed |
| 0011 | Hybrid GPU-driven rendering pipeline | Proposed — detailed in `Resident Representation/pipeline-stages.md` |
| 0012 | COOP/COEP headers + renderer worker architecture | Accepted |

## Document Status Registry

[`docs/CURRENT.md`](../docs/CURRENT.md) — **start here when navigating docs**. Every file in `docs/` classified as current / proposed / stale / legacy / research. Stale docs are explicitly flagged so they are not mistaken for current guidance.

## Resident Render Design (authoritative spec)

[`memory/Resident Representation/`](./Resident%20Representation/) — symlink to `docs/Resident Representation/`. The current authoritative GPU-resident architecture. Supersedes `gpu-driven-rendering/` design docs. Core files: `pipeline-stages.md` (authoritative pipeline), `layer-model.md` (three-product model), `chunk-contract.md`, `edit-protocol.md`, `meshlets.md`. **Note:** `ui-design-system.md`, `ui-migration.md`, `ui-interaction-design.md` in this dir are stale — see `docs/CURRENT.md`.

## Data Structure & Algorithm Map

[`memory/architecture-map.md`](./architecture-map.md) — symlink to `docs/architecture-map.md`. Complete data structure inventory (Tier 1–5), algorithm inventory (implemented + planned), shared dependency matrix, five pillars, coordinate systems, implementation priority P0–P10. High-value reference when working on any implementation task.

## GPU-Driven Rendering Plan

[`memory/gpu-driven-rendering.md`](./gpu-driven-rendering.md) — ADR-0010 (radiance cascades: hybrid screenspace probes + world-space voxel raymarch), ADR-0011 (custom WebGPU for chunks, Three.js for overlay), depth prepass as shared prerequisite, target frame pipeline, key constraints.

## Renderer Worker (implemented stub, 2026-03-21)

`apps/gestalt/src/renderer/` — binary protocol (`protocol.ts`), worker entry (`renderer.worker.ts`), main-thread bridge (`RendererBridge.ts`), Svelte store (`stores/rendererBridge.ts`). COOP/COEP headers set via coi-serviceworker for GitHub Pages. SABs allocated. WASM/WebGPU not yet wired — this is the landing zone for ADR-0011.

## Testing Infrastructure

[`memory/testing-infrastructure.md`](./testing-infrastructure.md) — Vitest for @gestalt/phi, Playwright E2E for apps/gestalt, wasm-bindgen-test for wasm_greedy_mesher, cargo test workspace. CI has four parallel jobs. WGSL/GPU testing deferred.

## Crate Lifecycle

[`memory/crate-deprecation-plan.md`](./crate-deprecation-plan.md) — crates graduate to `legacy/crates/` as WGSL compute shaders replace them. `greedy_mesher` survives longest as CPU oracle. Do NOT move crates prematurely — they are the correctness ground truth during the GPU rewrite.
