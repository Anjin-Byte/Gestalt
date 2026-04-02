# Legacy

Three.js + module-system era reference code. Do not add features here.

See [`apps/gestalt`](../apps/gestalt) for the active application.

## Contents

- `apps/web/` — Vite + Three.js viewer with the pluggable `ModuleHost` system
- `packages/modules/` — `@gestalt/modules` package (module contract + helpers)

## Run

From the repo root:

```bash
pnpm build:wasm:legacy
pnpm dev:legacy
```

Or use the convenience scripts:

```bash
./build.sh legacy
./run.sh legacy
```
