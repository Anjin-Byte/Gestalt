---
type: project
title: Rust Crate Deprecation Plan
date: 2026-03-21
---

# Rust Crate Deprecation Plan

The Rust crates in `crates/` are expected to be gradually deprecated as GPU features move to native WGSL compute shaders. Crates will graduate to `legacy/crates/` one-by-one as WGSL replacements are validated. Do NOT move prematurely — the Rust crates serve as correctness ground truth during the GPU rewrite.

## Crate Status

### `crates/greedy_mesher`
**Status:** Active — survives longest as CPU reference oracle.
- Contains `reference_cpu.rs` and extensive tests.
- Will be used to validate the WGSL greedy meshing compute shader implementation.
- Move to legacy only after the WGSL mesher is validated against it.

### `crates/voxelizer`
**Status:** Active — superseded in principle by native WGSL voxelization compute shaders.
- The WGSL voxelizer eliminates the WASM round-trip overhead.
- Deprecate when the WGSL voxelizer is running and producing correct output.

### `crates/wasm_greedy_mesher`
**Status:** Active — WASM bindings for the CPU mesher.
- Deprecated when the WGSL mesher is validated against `crates/greedy_mesher`.

### `crates/wasm_voxelizer`
**Status:** Active — WASM bindings for the CPU voxelizer.
- Likely one of the first to go, once the WGSL voxelizer is running.

### `crates/wasm_webgpu_demo`
**Status:** Active — prototype/demo crate.
- Superseded by the native custom WebGPU pipeline.
- Likely deprecated alongside `crates/wasm_voxelizer` as one of the first to go.

### `crates/wasm_obj_loader`
**Status:** Active — may survive longer than the rest.
- Handles CPU geometry parsing; the Rust parser is fast.
- No GPU equivalent is needed or planned.
- Retain until a clear reason to replace it emerges.

## Deprecation Order (expected)

1. `crates/wasm_webgpu_demo` — superseded by native pipeline
2. `crates/wasm_voxelizer` + `crates/voxelizer` — superseded by WGSL voxelizer
3. `crates/wasm_greedy_mesher` — superseded by WGSL mesher (after oracle validation)
4. `crates/greedy_mesher` — retired last, after WGSL mesher is fully validated
5. `crates/wasm_obj_loader` — indefinite; no planned replacement

## Process

- Validate WGSL replacement against Rust oracle before deprecating.
- Move deprecated crates to `legacy/crates/<crate-name>/` with a note in this file.
- Update ADRs as appropriate when a crate is retired.
