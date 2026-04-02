# Data Structure Specifications

**Type:** reference
**Status:** current
**Date:** 2026-03-22

> One document per GPU-resident data structure. Exact layouts, invariants, valid ranges, and validation strategy.

Each document answers: What are the bytes? What must always be true? How do you prove it?

---

## Authoritative (Producer-Written)

| Structure | Doc | Size per slot | Owner |
|---|---|---|---|
| Chunk Occupancy Atlas | [chunk-occupancy-atlas](chunk-occupancy-atlas.md) | 32 KB | Voxelizer / Edit kernels |
| Chunk Palette | [chunk-palette](chunk-palette.md) | Variable (≤512 B) | Voxelizer / Edit kernels |
| Chunk Index Buffer | [chunk-index-buf](chunk-index-buf.md) | Variable (≤32 KB) | Voxelizer / Edit kernels |
| Chunk Coord | [chunk-coord](chunk-coord.md) | 16 B | CPU slot manager |
| Chunk Version | [chunk-version](chunk-version.md) | 4 B | Edit kernels |
| Material Table | [material-table](material-table.md) | 16 B × MAX_MATERIALS | Scene manager |

## Derived (Rebuildable)

| Structure | Doc | Size per slot | Producer stage |
|---|---|---|---|
| Occupancy Summary | [occupancy-summary](occupancy-summary.md) | 64 B | I-3 |
| Chunk Flags | [chunk-flags](chunk-flags.md) | 4 B | I-3 |
| Chunk AABB | [chunk-aabb](chunk-aabb.md) | 32 B | I-3 |
| Vertex Pool | [vertex-pool](vertex-pool.md) | Variable | R-1 |
| Index Pool | [index-pool](index-pool.md) | Variable | R-1 |
| Draw Metadata | [draw-metadata](draw-metadata.md) | 32 B | R-1 |

## Per-Frame (Transient)

| Structure | Doc | Size | Producer stage |
|---|---|---|---|
| Depth Texture | [depth-texture](depth-texture.md) | W×H×4 B | R-2 |
| Hi-Z Pyramid | [hiz-pyramid](hiz-pyramid.md) | ~1.33× depth | R-3 |
| Indirect Draw Buffer | [indirect-draw-buf](indirect-draw-buf.md) | 20 B × MAX_DRAWS | R-4 |
| Cascade Atlas | [cascade-atlas](cascade-atlas.md) | ~64 MB per level | R-6 |
| Camera Uniform | [camera-uniform](camera-uniform.md) | 256 B | Per frame |

## Control Plane (Edit Protocol)

| Structure | Doc | Size | Owner |
|---|---|---|---|
| Dirty Chunks Bitset | [dirty-chunks](dirty-chunks.md) | 128 B (1024 bits) | Edit kernels |
| Stale Flags | [stale-flags](stale-flags.md) | 12 B per slot | Propagation pass |
| Rebuild Queues | [rebuild-queues](rebuild-queues.md) | 4 B × queue_len | Compaction pass |
