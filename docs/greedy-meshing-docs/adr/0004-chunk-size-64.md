# 0004 - 64³ Chunk Size

## Status
Accepted

## Context

The voxel mesh system divides the world into fixed-size chunks for:
- Incremental mesh rebuilds (only dirty chunks)
- Spatial locality for rendering
- Memory management and LOD

**Previous decision**: 32³ chunks were initially specified.

**New constraint**: The binary greedy meshing algorithm (ADR-0003) operates on 64-bit bitmasks, processing one bit per voxel along the Y-axis. This naturally aligns with 64-voxel columns.

**Trade-offs to consider**:

| Chunk Size | Pros | Cons |
|------------|------|------|
| 16³ | Small rebuild scope, fine-grained LOD | Too many chunks, high overhead |
| 32³ | Balanced rebuild scope | Doesn't align with 64-bit operations |
| 64³ | Perfect 64-bit alignment, fewer chunks | Larger rebuild scope, more memory per chunk |
| 128³ | Very few chunks | Rebuild scope too large, excessive memory |

## Decision

Use **64³ chunk size** (with 62³ usable voxels due to 1-voxel boundary padding).

**Rationale**:
1. **Algorithm alignment**: Binary greedy meshing uses `u64` bitmasks. 64³ means one column = one `u64`, enabling maximum bitwise parallelism.
2. **Memory efficiency**: Opaque mask is exactly 64×64×8 = 32KB, fitting in L1/L2 cache.
3. **Reasonable rebuild scope**: ~262K voxels per chunk, meshable in <100µs.
4. **Chunk count**: For typical scenes, results in manageable chunk counts.

**Usable size**: Due to the PaddedChunkView pattern for cross-chunk boundary handling, the usable interior is 62³ voxels (1-voxel padding on each face for neighbor lookups).

## Consequences

### Positive
- **Optimal bitwise operations**: One `u64` per column, no bit packing across boundaries
- **Cache-friendly**: 32KB opaque mask fits in L2 cache
- **Simpler indexing**: `mask[y * 64 + z]` with no boundary calculations
- **Fewer chunks**: Compared to 32³, 8x fewer chunks for same volume

### Negative
- **Larger rebuild scope**: Editing one voxel rebuilds up to 262K voxels worth of mesh
- **Memory per chunk**: ~288KB minimum (32KB mask + 256KB types)
- **Coarser LOD granularity**: LOD transitions happen at 64-voxel boundaries

### Memory Budget

| Data | Size per Chunk |
|------|----------------|
| Opaque mask | 32 KB |
| Material IDs | 256 KB |
| Face masks (temp) | 192 KB |
| Packed quads (temp) | Variable (~16 KB typical) |
| **Total working set** | **~500 KB** |

For a 16×16×16 chunk grid (4096 chunks), base storage is ~1.2 GB. With mesh data, expect 2-4 GB for large worlds.

### Migration from 32³

Documents updated to reflect 64³:
- [chunk-management-system.md](../chunk-management-system.md): `Chunk::SIZE = 64`
- [threejs-buffer-management.md](../threejs-buffer-management.md): `CHUNK_SIZE = 64`
- [voxel-mesh-architecture.md](../voxel-mesh-architecture.md): Memory layout tables

No code migration needed as implementation has not started.

## References
- [ADR-0003](0003-binary-greedy-meshing.md) - Binary greedy meshing (requires 64³)
- [voxel-mesh-architecture.md](../voxel-mesh-architecture.md#memory-layout) - Memory calculations
