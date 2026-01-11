# Gestalt WebGPU + WebAssembly Test Bed

A general-purpose WebGPU/WebAssembly test bed for visualizing module outputs without coupling to module internals. The host renders generic outputs (meshes, voxels, points, lines, textures) and can load WASM-backed modules.

## Repo layout

- `apps/web`: Vite + TypeScript viewer host
- `crates/*`: Rust crates (WASM modules)
- `packages/*`: shared TS types/schemas (optional)
- `.github/workflows/pages.yml`: GitHub Pages deployment
- `docs/adr`: architecture decision records

## Local development

```bash
pnpm install
pnpm dev
```

The viewer runs at `http://localhost:5173`.

## Build

```bash
pnpm build
```

Artifacts land in `apps/web/dist`.

## Module plugin interface

Modules implement `TestbedModule` in `apps/web/src/modules/types.ts` and return `ModuleOutput[]`. The host only consumes the interface types.

To add a new module:

1. Create a module in `apps/web/src/modules`.
2. Export it from `apps/web/src/modules/registry.ts`.
3. Implement `run()` to return outputs using typed arrays.

## WASM modules

The Rust example is in `crates/wasm_example` and uses wasm-pack for web ES modules.
There is also a spiral-points example in `crates/wasm_points`, an OBJ loader in `crates/wasm_obj_loader`, and a WebGPU demo in `crates/wasm_webgpu_demo`.

```bash
pnpm build:wasm
```

Or run manually:

```bash
cd crates/wasm_example
wasm-pack build --target web --out-dir ../../apps/web/src/wasm/wasm_example
```

The web host dynamically imports `wasm_example.js` from `apps/web/src/wasm/wasm_example`.
If you haven't run the wasm build, a small placeholder module is used so the app still boots.
The spiral points module loads from `apps/web/src/wasm/wasm_points`.
The OBJ loader module loads from `apps/web/src/wasm/wasm_obj_loader`.
The WebGPU demo module loads from `apps/web/src/wasm/wasm_webgpu_demo`.

## Deployment (GitHub Pages)

The GitHub Actions workflow builds the Vite app with `BASE_PATH=/<repo>/` and deploys `apps/web/dist` to Pages using the official actions:

- `actions/upload-pages-artifact`
- `actions/deploy-pages`
- The workflow also runs `pnpm build:wasm` so the example module is available on Pages.

Live site: `https://anjin-byte.github.io/Gestalt/`

## Rendering backend

The default backend is Three.js WebGPURenderer with WebGL2 fallback. The backend is wrapped so another renderer can be swapped in later.

## Tests

Playwright runs smoke + screenshot regression tests:

```bash
pnpm test:e2e
```

Tests run the app in deterministic mode with `?test=1`.

To update screenshots:

```bash
pnpm -C apps/web test:e2e --update-snapshots
```

## Known limitations / fallback behavior

- WebGPU is attempted first; when unavailable or failing to initialize, the viewer falls back to WebGL2.
- WASM modules must be built before they can be loaded by the host.

## Docs

- WebGPU profiling: `docs/PROFILING.md`
- ADRs: `docs/adr`
