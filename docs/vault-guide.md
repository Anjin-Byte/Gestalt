# Vault Guide

The Obsidian vault lives at `../Gestalt-vault/` alongside the repo (not committed to git).

---

## What goes in the vault

Research, design explorations, and reference materials that are useful to read in Obsidian but don't need to be in the repo:

- GUI design specs and research (outliner spec, Blender architecture analysis)
- Reference PDFs
- Deep-research outputs
- Historical design notes

## What stays in the repo (`docs/`)

ADRs, architecture specs, and implementation notes that should be versioned with the code:

- `docs/adr/` — all ADRs
- `docs/architecture/` — forward-looking specs

---

## Recommended vault structure

```
Gestalt-vault/
├── .obsidian/
├── 00-index.md
├── gui/
│   ├── gestalt-outliner-spec.md
│   ├── blender-outliner-architecture.md
│   ├── gpu-outliner-research.md
│   └── gpu-debugger-ui-gaps.md
├── renderer/
│   ├── Resident Representation/   (full tree)
│   ├── gpu-driven-rendering/      (design + spec dirs, not ADRs)
│   └── culling/
├── research/
│   ├── RadianceCascades.pdf
│   ├── deep-research-indirect.md
│   ├── deep-research-report.md
│   └── woo/
└── legacy/
    ├── greedy-meshing-docs/       (full tree)
    └── voxelizer-integration/     (full tree)
```

## Files to copy from repo root

These root-level markdown files belong in `vault/gui/`:

- `gestalt-outliner-spec.md` → `vault/gui/gestalt-outliner-spec.md`
- `blender-outliner-architecture.md` → `vault/gui/blender-outliner-architecture.md`

After copying to the vault they can be removed from the repo root.

## Files to copy from `docs/`

- `docs/research/RadianceCascades.pdf` → `vault/research/RadianceCascades.pdf`
- `docs/research/deep-research-*.md` → `vault/research/`
- `docs/research/woo/` → `vault/research/woo/`
- `docs/culling/` → `vault/renderer/culling/`
- `docs/Resident Representation/` → `vault/renderer/Resident Representation/`
- `docs/legacy/greedy-meshing-docs/` → `vault/legacy/greedy-meshing-docs/`
- `docs/voxelizer-integration/` → `vault/legacy/voxelizer-integration/`
- `docs/legacy/gpu-driven-rendering/design/`, `/spec/` → `vault/renderer/gpu-driven-rendering/`

ADRs (`docs/adr/`) and architecture specs (`docs/architecture/`) stay in the repo.
