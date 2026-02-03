# 0005 - Voxelizer to Mesher Integration

## Status
Proposed

## Context

The Gestalt project has two distinct systems that need to work together:

1. **GPU Voxelizer** (Implemented): Converts triangle meshes to sparse voxel data using WebGPU compute shaders
2. **Binary Greedy Mesher** (Documented): Converts voxel data to optimized triangle meshes using 64-bit bitwise operations

These systems were designed independently and have **incompatible data structures**.

### Current Implementation (GPU Voxelizer)

Location: `crates/voxelizer/src/`

```rust
pub struct SparseVoxelizationOutput {
    pub brick_dim: u32,              // Variable: 2-8 (GPU-calculated)
    pub brick_origins: Vec<[u32; 3]>, // Sparse brick positions
    pub occupancy: Vec<u32>,          // Bitpacked per brick
    pub owner_id: Option<Vec<u32>>,   // Triangle provenance
    pub color_rgba: Option<Vec<u32>>, // Visualization color
}
```

**Characteristics:**
- Variable brick size (2-8) based on GPU limits
- Sparse storage: only non-empty bricks stored
- Bitpacked occupancy per brick (not per column)
- `owner_id` tracks which triangle created each voxel
- Rendering: Points or InstancedMesh (one cube per voxel)

### Documented Design (Binary Greedy Mesher)

Location: `docs/greedy-mesh-implementation-plan.md`

```rust
pub struct BinaryChunk {
    pub opaque_mask: [u64; CS_P2],  // Column-based (64 voxels per u64)
    pub voxel_types: [u8; CS_P3],   // Material ID per voxel
}
```

**Characteristics:**
- Fixed 64³ chunk size (62³ usable with padding)
- Column-organized `u64` masks for 64-voxel parallel processing
- `voxel_types` for material-based greedy merging
- Rendering: Greedy-merged BufferGeometry mesh

### Incompatibility Summary

| Aspect | GPU Voxelizer | Binary Mesher | Gap |
|--------|---------------|---------------|-----|
| **Chunk size** | Variable (2-8)³ bricks | Fixed 64³ | 🔴 Critical |
| **Storage format** | Sparse brick + u32 bitpack | Dense column + u64 mask | 🔴 Critical |
| **Bit organization** | Per-brick | Per-Y-column | 🔴 Critical |
| **Material concept** | `owner_id` (triangle source) | `voxel_types` (game material) | 🟡 Semantic |
| **Rendering output** | Points / InstancedMesh | Greedy-merged Mesh | 🔴 Critical |

## Decision

Adopt a **staged integration approach** with a conversion layer:

### Stage 1: Conversion Function (Near-term)

Add a converter from sparse bricks to binary chunks:

```rust
/// Convert GPU voxelizer output to binary chunk format for greedy meshing
pub fn sparse_to_binary_chunks(
    sparse: &SparseVoxelizationOutput,
    grid_spec: &VoxelGridSpec,
) -> HashMap<ChunkCoord, BinaryChunk> {
    let mut chunks: HashMap<ChunkCoord, BinaryChunk> = HashMap::new();

    for (brick_idx, origin) in sparse.brick_origins.iter().enumerate() {
        // Determine which 64³ chunk(s) this brick overlaps
        let chunk_coords = get_overlapping_chunks(origin, sparse.brick_dim);

        for chunk_coord in chunk_coords {
            let chunk = chunks.entry(chunk_coord).or_insert_with(BinaryChunk::new);

            // Extract occupancy bits from brick and insert into chunk columns
            copy_brick_to_chunk(
                &sparse.occupancy,
                brick_idx,
                sparse.brick_dim,
                origin,
                chunk,
                chunk_coord,
            );
        }
    }

    chunks
}

/// Copy brick occupancy bits into chunk's column-based format
fn copy_brick_to_chunk(
    occupancy: &[u32],
    brick_idx: usize,
    brick_dim: u32,
    brick_origin: &[u32; 3],
    chunk: &mut BinaryChunk,
    chunk_coord: ChunkCoord,
) {
    let brick_voxels = brick_dim * brick_dim * brick_dim;
    let words_per_brick = (brick_voxels + 31) / 32;
    let base_word = brick_idx * words_per_brick as usize;

    // Chunk origin in world voxel coordinates
    let chunk_origin = [
        chunk_coord.x as u32 * 64,
        chunk_coord.y as u32 * 64,
        chunk_coord.z as u32 * 64,
    ];

    for local_z in 0..brick_dim {
        for local_y in 0..brick_dim {
            for local_x in 0..brick_dim {
                let bit_idx = local_z * brick_dim * brick_dim
                            + local_y * brick_dim
                            + local_x;
                let word_idx = base_word + (bit_idx / 32) as usize;
                let bit_in_word = bit_idx % 32;

                if (occupancy[word_idx] >> bit_in_word) & 1 != 0 {
                    // Convert to chunk-local coordinates
                    let world_x = brick_origin[0] + local_x;
                    let world_y = brick_origin[1] + local_y;
                    let world_z = brick_origin[2] + local_z;

                    let chunk_x = (world_x - chunk_origin[0]) as usize;
                    let chunk_y = (world_y - chunk_origin[1]) as usize;
                    let chunk_z = (world_z - chunk_origin[2]) as usize;

                    if chunk_x < 64 && chunk_y < 64 && chunk_z < 64 {
                        // Set bit in column-based opaque mask
                        let column_idx = chunk_x * 64 + chunk_z;
                        chunk.opaque_mask[column_idx] |= 1u64 << chunk_y;

                        // Set material (default to 1 for now)
                        let voxel_idx = chunk_x * 64 * 64 + chunk_y * 64 + chunk_z;
                        chunk.voxel_types[voxel_idx] = 1;
                    }
                }
            }
        }
    }
}
```

### Stage 2: Dual Rendering Modes (Medium-term)

Support both rendering approaches via a mode switch:

```typescript
type RenderMode = 'preview' | 'optimized';

interface VoxelRenderOptions {
  mode: RenderMode;
  // Preview mode: fast, per-voxel rendering
  previewStyle?: 'points' | 'cubes';
  // Optimized mode: greedy-meshed rendering
  greedyMeshEnabled?: boolean;
}
```

**Preview Mode** (current implementation):
- Uses GPU voxelizer output directly
- Points or InstancedMesh rendering
- Fast iteration during model exploration

**Optimized Mode** (new):
- Converts to BinaryChunks
- Applies greedy meshing
- Standard BufferGeometry mesh
- Better performance for final output

### Stage 3: Material Mapping (Future)

Bridge the semantic gap between `owner_id` and `voxel_types`:

```rust
/// Map triangle owner IDs to material types
pub struct MaterialMapper {
    /// Triangle ID → Material ID mapping
    triangle_to_material: Vec<u8>,
    /// Default material for unmapped triangles
    default_material: u8,
}

impl MaterialMapper {
    pub fn map_owner_to_material(&self, owner_id: u32) -> u8 {
        self.triangle_to_material
            .get(owner_id as usize)
            .copied()
            .unwrap_or(self.default_material)
    }
}
```

This allows:
- Assigning materials based on source mesh regions
- Preserving provenance while enabling material-based merging
- Future: per-triangle material assignment in input mesh

## Consequences

### Positive

- **Preserves existing work**: GPU voxelizer remains fully functional
- **Incremental adoption**: Can switch between modes as needed
- **Clear separation**: Voxelization and meshing remain decoupled
- **Performance flexibility**: Preview mode for iteration, optimized for final output

### Negative

- **Conversion overhead**: Extra pass to convert sparse → binary chunks
- **Memory duplication**: Both formats may exist simultaneously during conversion
- **Complexity**: Two code paths for rendering

### Constraints Introduced

- Conversion requires grid dimensions to be multiples of 64 for clean chunk boundaries
- Material mapping requires explicit configuration (no automatic inference)
- Preview and optimized modes may produce visually different results (acceptable)

## Alternatives Considered

### Alternative A: Modify GPU Voxelizer Output

Change the voxelizer to output column-based 64-bit masks directly.

**Rejected because:**
- Requires significant GPU shader changes
- Loses sparse storage benefits
- Variable brick size optimizes for GPU memory limits

### Alternative B: Modify Binary Mesher Input

Change the mesher to accept sparse brick format.

**Rejected because:**
- Breaks 64-bit column alignment (core optimization)
- Variable brick size prevents parallel processing
- Would require complete algorithm redesign

### Alternative C: Abandon Binary Meshing

Continue using only Points/InstancedMesh rendering.

**Rejected because:**
- O(n) geometry per voxel is not scalable
- No surface merging means excessive draw calls
- Documented performance requirements (REQ-PERF-*) cannot be met

## Implementation Plan

| Phase | Deliverable | Depends On |
|-------|-------------|------------|
| 1.1 | `sparse_to_binary_chunks()` function | — |
| 1.2 | Unit tests for conversion accuracy | 1.1 |
| 1.3 | WASM bindings for conversion | 1.2 |
| 2.1 | `RenderMode` enum and switch | 1.3 |
| 2.2 | UI toggle for preview/optimized | 2.1 |
| 3.1 | `MaterialMapper` struct | 2.2 |
| 3.2 | Per-triangle material input | 3.1 |

## References

- [GPU Voxelizer Implementation](../../crates/voxelizer/src/core.rs) - Current data structures
- [Binary Greedy Meshing Plan](../greedy-mesh-implementation-plan.md) - Target data structures
- [ADR-0003](0003-binary-greedy-meshing.md) - Binary greedy meshing algorithm decision
- [ADR-0004](0004-chunk-size-64.md) - 64³ chunk size decision
- [Architecture Addendum §2](../architecture-addendum.md#2-voxelizer--chunk-conversion) - Conversion approach
