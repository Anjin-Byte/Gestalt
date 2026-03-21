# Gestalt

A GPU-driven voxel mesh renderer built with Rust/WASM + Svelte 5 + WebGPU.

Live demo: [anjin-byte.github.io/Gestalt](https://anjin-byte.github.io/Gestalt/)

---

## Repo layout

```
apps/
  gestalt/          — active Svelte 5 app (pnpm dev targets this)
packages/
  phi/              — @gestalt/phi: Svelte 5 UI component kit
  voxelizer-js/     — TypeScript wrapper for the WASM voxelizer
crates/             — Rust crates (wasm-pack targets)
docs/
  adr/              — architecture decision records (0001–0011)
  architecture/     — forward-looking specs (WASM boundary protocol, etc.)
legacy/
  apps/web/         — Three.js + ModuleHost era (reference only)
  packages/modules/ — @gestalt/modules (retired)
```

## Local development

```bash
pnpm install
pnpm dev          # starts apps/gestalt at http://localhost:5173
```

## Build

```bash
pnpm build:wasm   # compile Rust crates → apps/gestalt/src/wasm/ (requires wasm-pack)
pnpm build        # Vite production build → apps/gestalt/dist/
```

Or use the convenience scripts:

```bash
./build.sh        # build:wasm (if wasm-pack available) + build
./run.sh          # pnpm dev
```

## WASM crates

| Crate | Purpose |
|-------|---------|
| `crates/wasm_obj_loader` | OBJ mesh loader |
| `crates/wasm_webgpu_demo` | WebGPU demo |
| `crates/wasm_voxelizer` | GPU mesh-to-voxel rasterizer |
| `crates/wasm_greedy_mesher` | Binary greedy meshing (64³ chunks) |

## Docs

- **ADRs** — `docs/adr/` (0001–0011, all architectural decisions)
- **WASM boundary protocol** — `docs/architecture/wasm-boundary-protocol.md`
- **Vault guide** — `docs/vault-guide.md` (what lives in the Obsidian vault vs. this repo)
