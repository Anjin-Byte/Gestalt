---
name: Testing Infrastructure
description: Complete testing stack for UI (Vitest/Playwright), Rust/WASM (wasm-bindgen-test), and CI configuration
type: project
---

## Testing Stack (established 2026-03-21)

### 1. packages/phi — Vitest unit tests

- **Config:** `packages/phi/vitest.config.ts`
- **Key setting:** `resolve.conditions: ['browser']` — prevents Svelte server build, forces client exports
- **Environment:** `happy-dom`
- **Setup file:** `packages/phi/src/test-setup.ts` — jest-dom + Web Animations API stub (Element.prototype.animate)
- **Tests:** `packages/phi/src/**/*.test.ts`
- **Run:** `pnpm -C packages/phi test`

**Known gotcha — Svelte slide transition:** happy-dom doesn't implement `element.animate()`. Svelte's `slide` transition calls it. The stub in test-setup.ts returns a minimal Animation-shaped object. Collapse assertions use `toHaveAttribute("inert")` on the container body, not `not.toBeInTheDocument()` — Svelte keeps exiting elements in DOM with `inert` during transitions.

**Tests written:** BarMeter.test.ts (11), StatusIndicator.test.ts (11), Section.test.ts (7) — all pass.

### 2. apps/gestalt — Playwright E2E tests

- **Config:** `apps/gestalt/playwright.config.ts`
- **Pattern:** `apps/gestalt/tests/*.spec.ts`
- **Base URL:** `http://127.0.0.1:4173` (Vite preview server)
- **Data-ready signal:** `App.svelte` sets `document.body.dataset.ready = "true"` in `$effect`. Tests use `waitForSelector("body[data-ready='true']")`.
- **Tests:** `apps/gestalt/tests/basic.spec.ts` — 8 smoke tests (app loads, panel tabs, collapse)
- **Run:** `pnpm -C apps/gestalt test:e2e`

### 3. crates/wasm_greedy_mesher — wasm-bindgen-test

- **Config:** `crates/wasm_greedy_mesher/Cargo.toml` has `wasm-bindgen-test = "0.3"` in dev-dependencies
- **Tests:** in `crates/wasm_greedy_mesher/src/lib.rs` — `#[cfg(test)] mod tests` with `wasm_bindgen_test_configure!(run_in_browser)`
- **8 tests:** empty grid → empty mesh, solid grid → geometry, single voxel → 12 triangles, UV on/off, empty position list, single position
- **Run:** `wasm-pack test --headless --chrome crates/wasm_greedy_mesher`

### 4. Rust — cargo test

- `cargo test --workspace` runs native Rust tests across all crates
- `greedy_mesher` has a `reference_cpu.rs` used as correctness oracle for future WGSL validation

## CI — .github/workflows/tests.yml

Four parallel jobs:

| Job | Command | Trigger |
|---|---|---|
| `phi-test` | `pnpm -C packages/phi test` | push/PR |
| `cargo-test` | `cargo test --workspace` | push/PR |
| `wasm-test` | `wasm-pack test --headless --chrome crates/wasm_greedy_mesher` | push/PR |
| `playwright` | `pnpm -C apps/gestalt test:e2e` | push/PR |

`wasm-test` installs Playwright Chromium via `npx playwright install chromium --with-deps` (wasm-bindgen-test drives the browser binary directly, not through Playwright's test runner).

## What's Deferred

GPU/WGSL testing deferred to a future session. Recommended approach when ready: wgpu + LLVMpipe software rasterizer on Linux for compute shader CI. `reference_cpu.rs` in greedy_mesher provides the CPU oracle for validating WGSL meshing output.
