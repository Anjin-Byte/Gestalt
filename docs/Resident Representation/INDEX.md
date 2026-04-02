# GPU Resident Architecture — Vault Index

**Type:** reference
**Status:** current
**Date:** 2026-03-21

This vault defines the canonical runtime voxel representation and the GPU-native rendering architecture built on top of it.

---

## The Three-Sentence Summary

> The engine should unify around how voxel space is **queried after it exists**, not around how it is produced. Choose the canonical data model based on the queries the runtime must answer repeatedly, not on the convenience of any one content-generation path. GPU-driven scheduling simplifies the control plane more than the storage plane.


---

## Core Design Documents

| Document                   | Status   | Description                                                                                                  |
| -------------------------- | -------- | ------------------------------------------------------------------------------------------------------------ |
| [chunk-contract](chunk-contract.md)         | **Done** | Narrative chunk contract — layers, edit semantics, residency protocol                                        |
| [chunk-field-registry](chunk-field-registry.md)   | **Done** | Explicit field matrix — authoritative/derived/rebuildable/GPU/traversal/meshing                              |
| [layer-model](layer-model.md)            | **Done** | Three-product architecture: world-space / surface / camera-visibility                                        |
| [pipeline-stages](pipeline-stages.md)        | **Done** | GPU stage diagram — exact buffers, textures, read/write ownership per stage                                  |
| [gpu-chunk-pool](gpu-chunk-pool.md)         | **Done** | GPU slot allocation, atlas layout, CPU↔GPU sync protocol                                                     |
| [traversal-acceleration](traversal-acceleration.md) | **Done** | Three-level DDA design, column-aware inner loop, traversal contract                                          |
| [extension-seams](extension-seams.md)        | **Done** | Architectural principles — integration test, invariants, layer model, extensibility framework                |
| [edit-protocol](edit-protocol.md)          | **Done** | Four-responsibility edit pipeline: change detection, staleness propagation, version tagging, work scheduling |
| [meshlets](meshlets.md)               | **Done** | Sub-chunk surface cluster tier: descriptor layout, pool design, two-phase R‑4 cull, edit invalidation        |
| [material-system](material-system.md)        | **Done** | Global material table, MaterialEntry layout, palette protocol, emissive invalidation                         |

## Renderer Documents

| Document | Status | Description |
|---|---|---|
| [demo-renderer](demo-renderer.md) | **Done** | Isolated demo module: custom WebGPU renderer, no Three.js |
| [radiance-cascades-impl](radiance-cascades-impl.md) | **Done** | Cascade build / merge / apply passes grounded in resident chunks |
| [depth-prepass](depth-prepass.md) | **Done** | Raster optimization chain: front-to-back sort → depth prepass → Hi-Z culling |
| [debug-profiling](debug-profiling.md) | **Done** | Timestamp query infrastructure, pass timeline visualization, debug render modes, diagnostic counters, five-tier testing strategy |
| [material-aware-merge](material-aware-merge.md) | **Done** | Per-voxel material identity in greedy merge: on-demand lookup design, register pressure analysis, bitpacking decode, performance model |
| [variable-mesh-pool](variable-mesh-pool.md) | **Done** | Variable-size freelist allocator for vertex/index pools, replacing fixed per-slot allocation |

## UI Documents

| Document | Status | Description |
|---|---|---|
| [ui-design-system](ui-design-system.md) | **Done** | Viaduct UI design system: Svelte 5 + Tailwind 4 + Bits UI, OKLCH color tokens, glassmorphism, component reference |
| [ui-migration](ui-migration.md) | **Done** | Migration plan: panel architecture, uiApi store bridge, dependency changes, five build phases |
| [ui-interaction-design](ui-interaction-design.md) | **Done** | Interaction design language: Blender-inspired scrub fields, spatial grammar, component contracts, status bar, implementation priority |

## Strict Specifications

Formal specifications with exact byte layouts, numbered invariants, pre/postconditions, and testing strategies.

| Directory | Contents | Doc count |
|---|---|---|
| [data/](data/INDEX.md) | One doc per GPU-resident data structure (authoritative, derived, per-frame, control plane) | 20 |
| [stages/](stages/INDEX.md) | One doc per pipeline stage (I-1 through I-3, R-1 through R-9) | 12 |
| [tests/](tests/INDEX.md) | Consistency tests: data invariants, stage contracts, cross-cutting properties | 11 |

---

## Reference

- [ADR-0013](../adr/0013-full-webgpu-worker-pipeline.md) — Full WebGPU pipeline in renderer worker (supersedes ADR-0011)
- [ADR-0010](../adr/0010-radiance-cascades.md) — Radiance cascades decision
- [Amanatides & Woo](../research/woo/Amanatides_and_Woo.md) — DDA traversal reference
- [gpu-driven-rendering INDEX](../legacy/gpu-driven-rendering/INDEX.md) — Legacy hybrid pipeline architecture (ADR-0011, superseded)

---