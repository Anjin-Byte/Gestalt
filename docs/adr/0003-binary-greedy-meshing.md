# 0003 - Binary Greedy Meshing Algorithm

## Status
Accepted

## Context

The voxel mesh system requires converting voxel data into renderable triangle meshes. The initial approach used a traditional per-voxel iteration with neighbor lookups, which proved to be a performance bottleneck.

**Problem**: Traditional greedy meshing algorithms process voxels one at a time with 6 neighbor lookups per voxel. For a 64³ chunk (262,144 voxels), this results in millions of memory accesses and branch predictions per mesh rebuild.

**Requirements**:
- Mesh rebuild time must be <200µs to support real-time editing
- Must handle arbitrary voxel configurations (terrain, caves, structures)
- Must support multiple materials with correct face merging

**Alternatives Considered**:

1. **Traditional Greedy Meshing**: O(n) per-voxel iteration with neighbor lookups
   - Pros: Simple to implement, well-documented
   - Cons: 6 memory lookups per voxel, poor cache locality, ~500-2000µs per chunk

2. **GPU Compute Meshing**: Offload to WebGPU compute shaders
   - Pros: Massive parallelism
   - Cons: WGSL lacks 64-bit integers, data transfer overhead, complexity

3. **Binary Greedy Meshing**: Bitwise operations on 64-bit masks
   - Pros: Process 64 voxels per instruction, excellent cache locality
   - Cons: More complex implementation, requires 64³ chunk size

## Decision

Adopt **Binary Greedy Meshing** using 64-bit bitmask operations in Rust/WASM.

**Key techniques**:
- Store voxel occupancy as `[u64; 64*64]` column bitmasks (one bit per voxel in Y)
- Face culling via bitwise AND/XOR with shifted neighbors
- Greedy merge using `trailing_zeros()` for bit scanning
- 64-bit packed quad format for intermediate storage

**Implementation outline**:
```rust
// Face culling: 64 voxels at once
let visible_top = column & !(column >> 1);  // +Y faces
let visible_bottom = column & !(column << 1);  // -Y faces

// Greedy merge via bit scanning
while mask != 0 {
    let start = mask.trailing_zeros();
    let run = (!mask >> start).trailing_zeros();
    // Emit quad from start with width=run
    mask &= !(((1u64 << run) - 1) << start);
}
```

## Consequences

### Positive
- **10-50x speedup**: Typical terrain chunk meshes in ~74µs vs ~500-2000µs
- **Cache-friendly**: Sequential memory access patterns
- **Deterministic**: Same input always produces same output
- **SIMD-like**: Effective 64-wide parallelism without actual SIMD

### Negative
- **Fixed chunk size**: Algorithm is optimized for 64³ chunks specifically
- **Implementation complexity**: Bitwise operations are harder to debug
- **64-bit requirement**: Cannot easily port to 32-bit systems (not a concern for WASM)

### Constraints Introduced
- Chunk size must be 64³ (see ADR-0004)
- Material comparison requires separate pass or packed encoding
- Cross-chunk boundaries require 1-voxel padding (PaddedChunkView)

## References
- [binary-greedy-meshing-analysis.md](../binary-greedy-meshing-analysis.md) - Detailed algorithm analysis
- [greedy-mesh-implementation-plan.md](../greedy-mesh-implementation-plan.md) - Full implementation spec
- [0xFFFF blog on binary meshing](https://0fps.net/2012/06/30/meshing-in-a-minecraft-game/) - Original inspiration
