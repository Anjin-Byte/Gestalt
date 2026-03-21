# Document Status Registry

This file is the authoritative index of every document in `docs/` and its current status.

**How to use:**
- `current` — read this when building. Authoritative.
- `proposed` — planned but not yet implemented. Valid design intent.
- `stale` — do NOT read as current guidance. Superseded or abandoned.
- `legacy` — archaeology only. Explains why old code was written the way it was.
- `research` — reference material. Not architecture specs.

When in doubt about whether a doc is still valid, check here first.

---

## Resident Representation/

The GPU-resident voxel runtime architecture. All documents finalized.

| File | Status | Notes |
|---|---|---|
| `INDEX.md` | current | Hub — start here |
| `layer-model.md` | current | Three-product model (occupancy / surface / visibility) |
| `chunk-contract.md` | current | Canonical chunk fields and ownership rules |
| `chunk-field-registry.md` | current | Authoritative/derived/rebuildable/GPU-resident classification |
| `pipeline-stages.md` | current | **Authoritative pipeline spec.** Exact buffer/texture ownership per stage. Supersedes `gpu-driven-rendering/` design docs. |
| `gpu-chunk-pool.md` | current | Slot allocation, atlas layout, CPU↔GPU sync |
| `meshlets.md` | current | Sub-chunk surface clusters, two-phase R-4 dispatch |
| `edit-protocol.md` | current | Change detection, staleness propagation, version tagging, work scheduling |
| `material-system.md` | current | Global material table, MaterialEntry layout, per-chunk palette |
| `depth-prepass.md` | current | Depth prepass + Hi-Z raster optimization chain |
| `traversal-acceleration.md` | current | Three-level DDA for world-space ray traversal |
| `radiance-cascades-impl.md` | current | Cascade build/merge/apply grounded in GPU-resident chunks |
| `extension-seams.md` | current | Principles for integrating future features without breaking canonical model |
| `debug-profiling.md` | current | GPU timing, diagnostic counters, state visibility |
| `demo-renderer.md` | current | Isolated demo module for architecture validation |
| `ui-design-system.md` | **stale** | Describes "Viaduct" — replaced by `@gestalt/phi` + Tailwind 4 setup |
| `ui-migration.md` | **stale** | Migration plan — was executed differently (workspace reorganization 2026-03-21) |
| `ui-interaction-design.md` | **stale** | UX research from an earlier design pass; interaction patterns have evolved |

---

## adr/

Architecture Decision Records. Accepted = implemented. Proposed = planned. Superseded = replaced.

| File | Status | Notes |
|---|---|---|
| `0003-binary-greedy-meshing.md` | current | Accepted — defines the u64 column bitmask layout |
| `0004-chunk-size-64.md` | current | Accepted — CS=62, CS_P=64 |
| `0009-architecture-b.md` | current | Accepted — GPU-compact voxelizer integration |
| `0012-coop-coep-renderer-worker.md` | current | Accepted — COOP/COEP headers, renderer worker, WASM chunk splitting |
| `0010-radiance-cascades.md` | proposed | Hybrid screenspace probes + world-space raymarch; implementation detail in `Resident Representation/radiance-cascades-impl.md` |
| `0011-hybrid-gpu-driven.md` | proposed | Hybrid pipeline decision; detailed implementation in `Resident Representation/pipeline-stages.md` |
| `0006-lod-strategy.md` | proposed | Point mode for distant chunks |
| `0007-material-strategy.md` | proposed | Texture atlas material system |
| `0008-design-gap-mitigations.md` | **stale** | Mitigations for Three.js gaps — gaps addressed by ADR-0011 |
| `0001-renderer-choice.md` | **stale** | Original Three.js choice — amended by ADR-0011, Three.js now overlay-only |
| `0002-module-contract.md` | **stale** | Module system — now in `legacy/packages/modules/` |
| `0005-voxelizer-to-mesher-integration.md` | **stale** | Superseded by ADR-0009 |

---

## architecture/

| File | Status | Notes |
|---|---|---|
| `wasm-boundary-protocol.md` | current | Binary command protocol, SAB ring buffer layout. Partially implemented (stub in place). |

---

## Root-level docs/

| File | Status | Notes |
|---|---|---|
| `architecture-map.md` | current | **High-value reference.** Complete data structure inventory, algorithm inventory, shared dependency matrix, five pillars, implementation priority P0–P10. |
| `README.md` | current | Docs index |
| `vault-guide.md` | current | What to copy to `../Gestalt-vault/` |

---

## culling/

| File | Status | Notes |
|---|---|---|
| `hiz-occlusion-culling-report.md` | proposed | Hi-Z readiness analysis — still valid future work, shared prerequisite with cascades |

---

## voxelizer-integration/

Documents the ADR-0009 GPU-compact integration. Core design docs still valid for implementation.

| File | Status | Notes |
|---|---|---|
| `philosophy.md` | current | "Chunk manager is the canonical voxel store" — still the core invariant |
| `design/cpu-ingestion.md` | proposed | What CPU does with GPU compact output — still needed for ADR-0009 impl |
| `design/gpu-output-contract.md` | proposed | What GPU compact pass must produce — still needed |
| `spec/invariants.md` | current | Four chunk invariants (C1–C4) — always valid |
| `spec/coordinate-frames.md` | current | World/voxel/chunk/local coordinate conversions |
| `spec/material-pipeline.md` | proposed | Material table packing — needed for ADR-0009 impl |
| `spec/wasm-api.md` | **stale** | WASM API for old integration approach — superseded |
| `design/requirements.md` | **stale** | Requirements for old approach |
| `impl/` | proposed | GPU shader changes, crate structure — needed for ADR-0009 impl |
| `archive/` | legacy | All superseded plans — archaeology only |
| `INDEX.md` | **stale** | Superseded hub |

---

## legacy/

Stale and superseded design documents. Explains why old code was written the way it was. Do not read as current guidance.

### legacy/gpu-driven-rendering/

Predates `Resident Representation/`. Superseded by `pipeline-stages.md`.

| File | Status | Notes |
|---|---|---|
| `design/three-js-limits.md` | legacy | Evidence for why Three.js is insufficient — explains ADR-0011 rationale |
| `spec/visibility-buffer.md` | proposed | Future meshlet/visibility buffer — not yet in Resident Representation |
| All others | **stale** | Superseded by `Resident Representation/pipeline-stages.md` |

### legacy/greedy-meshing-docs/

Original Rust greedy mesher design. Algorithms are fully implemented.

| File | Status | Notes |
|---|---|---|
| All files | legacy | Explains implemented algorithms and old TS architecture |

### legacy/implementation-status.md

| File | Status | Notes |
|---|---|---|
| `implementation-status.md` | **stale** | Requirements scorecard is against old Three.js codebase |

---

## research/

Reference material. Not architecture specs.

| File | Status | Notes |
|---|---|---|
| `RadianceCascades.pdf` | research | Sannikov paper on radiance cascades |
| `deep-research-indirect.md` | research | Indirect rendering research |
| `deep-research-report.md` | research | Deep research report |
| `deep-research-report_response.md` | research | Response to deep research |
| `woo/Amanatides_and_Woo.md` | research | DDA paper notes — reference for `traversal-acceleration.md` |
