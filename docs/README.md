# docs/

Technical documentation for the Gestalt project.

**Not sure where to look?** Start with [`CURRENT.md`](CURRENT.md) — every file in this tree with a status label (current / proposed / stale / legacy / research).

---

## Current architecture

| Path | Contents |
|---|---|
| `Resident Representation/` | Authoritative GPU-resident spec — chunk contract, pipeline stages, layer model, meshlets, edit protocol, material system, GI |
| `adr/` | Architecture Decision Records 0001–0012 |
| `architecture/` | Implementation specs (WASM boundary protocol) |
| `architecture-map.md` | Complete data structure + algorithm inventory, shared dependency matrix, Five Pillars, implementation priority P0–P10 |
| `culling/` | Hi-Z occlusion culling readiness report (proposed future work) |
| `voxelizer-integration/` | ADR-0009 GPU-compact integration design and spec (proposed) |

## Legacy — archaeology only

`legacy/` contains design documents that explain why the old codebase was the way it was. Do not read as current guidance.

| Path | Contents |
|---|---|
| `legacy/greedy-meshing-docs/` | Original Rust greedy mesher design |
| `legacy/gpu-driven-rendering/` | Early GPU pipeline design — superseded by `Resident Representation/pipeline-stages.md` |
| `legacy/implementation-status.md` | Requirements scorecard against old Three.js architecture |

## Research — reference material

`research/` contains papers and deep-research reports. Not architecture specs.

| Path | Contents |
|---|---|
| `research/RadianceCascades.pdf` | Sannikov paper on radiance cascades |
| `research/deep-research-*.md` | GPU indirect rendering and technique research |
| `research/woo/` | Amanatides & Woo DDA paper notes |

---

See `vault-guide.md` for what belongs in `../Gestalt-vault/` (Obsidian).
