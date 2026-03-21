# Module System Lifecycle

This document defines the intended lifecycle and ownership model for testbed modules.

## Goals

- Keep module logic isolated from host/viewer plumbing.
- Make module switching safe and deterministic.
- Prevent stale async results and resource leaks.
- Support both local dev and GitHub Pages deployments.

## Current Contract

`TestbedModule` now supports:

- `init(ctx)`: one-time initialization. Load code/resources that can be reused.
- `activate?(ctx)`: optional per-activation setup.
- `ui?(api)`: register module controls.
- `run(job)`: execute with current params and return outputs.
- `deactivate?()`: optional per-deactivation cleanup.
- `dispose?()`: final cleanup when host is disposed.

`RunRequest` now includes:

- `signal`: abort signal for cancelling stale runs.
- `moduleId`: active module id for tracing/debug.

## Host Lifecycle Rules

1. Module init is lazy: only when first activated.
2. Switching modules calls `deactivate` on the previous module.
3. In-flight runs are aborted on module switch.
4. Stale run results are ignored if activation changed.
5. UI state is cleared between modules (DOM + stored values).
6. `dispose` is only for final teardown (page unload / host shutdown).

## Resource Ownership

- Module-owned resources:
  - workers
  - wasm wrappers/adapters
  - per-module GPU pipelines/buffers
  - timers/event subscriptions started by the module
- Viewer-owned resources:
  - scene objects produced from `ModuleOutput`
  - geometry/material/texture disposal for rendered outputs
- Host-owned resources:
  - active run cancellation
  - module activation state
  - shared module UI container

## Suggested Module Layout

When a module grows, keep code in a module-local folder:

- `apps/web/src/modules/<module-id>/index.ts` (module factory)
- `apps/web/src/modules/<module-id>/types.ts`
- `apps/web/src/modules/<module-id>/runtime/*` (worker/client/interop glue)
- `apps/web/src/modules/<module-id>/ui/*`
- `apps/web/src/modules/<module-id>/utils/*`

Avoid scattering one module across unrelated top-level folders unless the code is truly shared.

## Migration Guidance

For existing modules:

1. Move one-time heavy setup into `init`.
2. Move switch-time cleanup into `deactivate`.
3. Keep `dispose` as final shutdown only.
4. Check `job.signal.aborted` in long async runs when possible.
5. Destroy temporary GPU buffers after each run.
