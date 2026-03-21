# GPU Resident Architecture — Vault Index

This vault defines the canonical runtime voxel representation and the GPU-native rendering architecture built on top of it.

---

## The Three-Sentence Summary

> The engine should unify around how voxel space is **queried after it exists**, not around how it is produced. Choose the canonical data model based on the queries the runtime must answer repeatedly, not on the convenience of any one content-generation path. GPU-driven scheduling simplifies the control plane more than the storage plane.


---

## Core Design Documents

| Document                   | Status   | Description                                                                                                  |
| -------------------------- | -------- | ------------------------------------------------------------------------------------------------------------ |
| [[chunk-contract]]         | **Done** | Narrative chunk contract — layers, edit semantics, residency protocol                                        |
| [[chunk-field-registry]]   | **Done** | Explicit field matrix — authoritative/derived/rebuildable/GPU/traversal/meshing                              |
| [[layer-model]]            | **Done** | Three-product architecture: world-space / surface / camera-visibility                                        |
| [[pipeline-stages]]        | **Done** | GPU stage diagram — exact buffers, textures, read/write ownership per stage                                  |
| [[gpu-chunk-pool]]         | **Done** | GPU slot allocation, atlas layout, CPU↔GPU sync protocol                                                     |
| [[traversal-acceleration]] | **Done** | Three-level DDA design, column-aware inner loop, traversal contract                                          |
| [[extension-seams]]        | **Done** | Architectural principles — integration test, invariants, layer model, extensibility framework                |
| [[edit-protocol]]          | **Done** | Four-responsibility edit pipeline: change detection, staleness propagation, version tagging, work scheduling |
| [[meshlets]]               | **Done** | Sub-chunk surface cluster tier: descriptor layout, pool design, two-phase R‑4 cull, edit invalidation        |
| [[material-system]]        | **Done** | Global material table, MaterialEntry layout, palette protocol, emissive invalidation                         |

## Renderer Documents

| Document | Status | Description |
|---|---|---|
| [[demo-renderer]] | **Done** | Isolated demo module: custom WebGPU renderer, no Three.js |
| [[radiance-cascades-impl]] | **Done** | Cascade build / merge / apply passes grounded in resident chunks |
| [[depth-prepass]] | **Done** | Raster optimization chain: front-to-back sort → depth prepass → Hi-Z culling |
| [[debug-profiling]] | **Done** | Timestamp query infrastructure, pass timeline visualization, debug render modes, diagnostic counters, five-tier testing strategy |

## UI Documents

| Document | Status | Description |
|---|---|---|
| [[ui-design-system]] | **Done** | Viaduct UI design system: Svelte 5 + Tailwind 4 + Bits UI, OKLCH color tokens, glassmorphism, component reference |
| [[ui-migration]] | **Done** | Migration plan: panel architecture, uiApi store bridge, dependency changes, five build phases |
| [[ui-interaction-design]] | **Done** | Interaction design language: Blender-inspired scrub fields, spatial grammar, component contracts, status bar, implementation priority |

## Reference

- [[../gpu-driven-rendering/INDEX]] — ADR-0011 hybrid pipeline architecture
- [[../greedy-meshing-docs/adr/0010-radiance-cascades]] — ADR-0010 radiance cascades decision
- [[../woo/Amanatides_and_Woo]] — DDA traversal reference

---