# 0006 - Level of Detail (LOD) Strategy

## Status
Proposed

## Context

The voxel mesh system needs to render large worlds efficiently. At distance, full-resolution greedy-meshed chunks are wasteful:
- GPU overdraw for sub-pixel geometry
- Memory consumption for invisible detail
- Mesh rebuild time for chunks the player may never approach

**Current architecture constraints:**
- 64³ chunks (ADR-0004) provide coarse LOD granularity
- Binary greedy meshing (ADR-0003) is optimized for 64-bit column operations
- GPU voxelizer already outputs sparse brick data
- Current renderer supports both `points` and `cubes` modes

**Requirements:**
- Maintain 60 FPS with 10+ chunk render distance
- Smooth transitions (no jarring "pop-in")
- Memory budget compliance
- Minimal implementation complexity for Phase 5

## Options Evaluated

### Option A: Voxel Decimation

Reduce voxel resolution before meshing: 64³ → 32³ → 16³ → 8³

```
LOD 0: 64³ voxels → binary greedy mesh (full detail)
LOD 1: 32³ voxels → binary greedy mesh (1/8 voxels)
LOD 2: 16³ voxels → binary greedy mesh (1/64 voxels)
LOD 3:  8³ voxels → binary greedy mesh (1/512 voxels)
```

**Implementation:**

```rust
/// Decimate chunk by averaging 2x2x2 blocks
fn decimate_chunk(chunk: &BinaryChunk) -> BinaryChunk {
    let mut result = BinaryChunk::new_size(32);

    for x in 0..32 {
        for y in 0..32 {
            for z in 0..32 {
                // Sample 2x2x2 block, use majority vote
                let count = count_solid_in_block(chunk, x*2, y*2, z*2, 2);
                if count >= 4 {  // Majority threshold
                    let material = dominant_material(chunk, x*2, y*2, z*2, 2);
                    result.set(x, y, z, material);
                }
            }
        }
    }

    result
}
```

**Pros:**
- Preserves voxel aesthetic at all distances
- Greedy meshing still works (fewer quads)
- Predictable memory reduction

**Cons:**
- Binary meshing requires 64-column alignment—32³/16³ breaks the optimization
- Need separate meshing paths or pad to 64
- LOD storage doubles memory if pre-computed
- Decimation artifacts (thin features disappear)

**Verdict:** ⚠️ Possible but requires meshing algorithm changes

---

### Option B: Mesh Simplification

Run quadric error mesh simplification on greedy mesh output.

```
LOD 0: Greedy mesh → 100% triangles
LOD 1: Greedy mesh → simplify to 25% triangles
LOD 2: Greedy mesh → simplify to 6% triangles
LOD 3: Greedy mesh → simplify to 1.5% triangles
```

**Implementation:**

```rust
/// Simplify mesh using quadric error metrics
fn simplify_mesh(mesh: &MeshOutput, target_ratio: f32) -> MeshOutput {
    // Use meshopt or similar library
    let target_indices = (mesh.indices.len() as f32 * target_ratio) as usize;
    meshopt::simplify(&mesh.positions, &mesh.indices, target_indices)
}
```

**Pros:**
- Works on any mesh (not voxel-specific)
- Existing libraries (meshopt, simplify)
- Can target specific triangle counts

**Cons:**
- Loses voxel grid alignment (rounded corners)
- May create T-junctions at chunk boundaries
- Simplification is O(n log n), adds to mesh time
- Visual style inconsistency at LOD boundaries

**Verdict:** ❌ Poor fit for voxel aesthetic

---

### Option C: Point Mode for Distant Chunks

Use existing point rendering mode as lowest LOD level.

```
LOD 0: Greedy mesh (close)
LOD 1: Greedy mesh (medium) — optional intermediate
LOD 2: Point cloud (far)
```

**Implementation:**

```typescript
interface ChunkLOD {
  mesh?: THREE.Mesh;        // Greedy-meshed geometry
  points?: THREE.Points;    // Point cloud fallback
  currentLOD: 'mesh' | 'points';
}

function updateChunkLOD(chunk: ChunkLOD, distance: number): void {
  const LOD_THRESHOLD = 256;  // voxels (4 chunks)

  if (distance > LOD_THRESHOLD && chunk.currentLOD === 'mesh') {
    // Switch to points
    chunk.mesh.visible = false;
    chunk.points.visible = true;
    chunk.currentLOD = 'points';
  } else if (distance <= LOD_THRESHOLD && chunk.currentLOD === 'points') {
    // Switch to mesh
    chunk.points.visible = false;
    chunk.mesh.visible = true;
    chunk.currentLOD = 'mesh';
  }
}
```

**Point generation options:**

```rust
/// Option 1: Surface points only (from greedy mesh vertices)
fn mesh_to_points(mesh: &MeshOutput) -> Vec<f32> {
    // Use quad centers, not all vertices
    let mut points = Vec::new();
    for i in (0..mesh.indices.len()).step_by(6) {
        // Average the 4 vertices of each quad
        let center = compute_quad_center(mesh, i);
        points.extend_from_slice(&center);
    }
    points
}

/// Option 2: Sparse voxel centers (from BinaryChunk)
fn chunk_to_points(chunk: &BinaryChunk, voxel_size: f32) -> Vec<f32> {
    let mut points = Vec::new();
    for x in 1..63 {
        for z in 1..63 {
            let column = chunk.opaque_mask[x * 64 + z];
            let mut bits = column;
            while bits != 0 {
                let y = bits.trailing_zeros() as usize;
                points.extend_from_slice(&[
                    x as f32 * voxel_size,
                    y as f32 * voxel_size,
                    z as f32 * voxel_size,
                ]);
                bits &= bits - 1;  // Clear lowest bit
            }
        }
    }
    points
}

/// Option 3: Decimated sparse points (every Nth voxel)
fn chunk_to_sparse_points(chunk: &BinaryChunk, voxel_size: f32, step: usize) -> Vec<f32> {
    let mut points = Vec::new();
    for x in (1..63).step_by(step) {
        for z in (1..63).step_by(step) {
            let column = chunk.opaque_mask[x * 64 + z];
            let mut bits = column;
            while bits != 0 {
                let y = bits.trailing_zeros() as usize;
                if y % step == 0 {
                    points.extend_from_slice(&[
                        x as f32 * voxel_size,
                        y as f32 * voxel_size,
                        z as f32 * voxel_size,
                    ]);
                }
                bits &= bits - 1;
            }
        }
    }
    points
}
```

**Pros:**
- Already implemented in current renderer (`renderMode: "points"`)
- Zero mesh simplification overhead
- Massive triangle reduction (points = 0 triangles)
- Natural voxel aesthetic (points look like distant voxels)
- Fast to generate from bitmask (no meshing needed)
- Memory efficient (just positions, no indices)

**Cons:**
- Visual discontinuity at LOD transition
- Point size scaling needed for consistent appearance
- No surface normals (flat shading only)
- May look sparse for hollow structures

**Verdict:** ✅ Recommended for Phase 5 MVP

---

### Option D: Hierarchical Chunks (Octree)

Group 2×2×2 chunks into "super-chunks" at each LOD level.

```
LOD 0: 64³ chunks (1 chunk = 1 mesh)
LOD 1: 128³ super-chunks (8 chunks merged, decimated to 64³)
LOD 2: 256³ super-chunks (64 chunks merged, decimated to 64³)
```

**Pros:**
- Maintains 64³ alignment at all levels
- Natural spatial hierarchy
- Efficient frustum culling

**Cons:**
- Requires hierarchical storage not currently designed
- Chunk boundary handling becomes complex
- Pre-computation of all LOD levels expensive
- Significant architectural change

**Verdict:** ❌ Too complex for Phase 5, consider for future

---

## Decision

Adopt **Option C: Point Mode for Distant Chunks** as the Phase 5 LOD implementation.

### LOD Configuration

```typescript
interface LODConfig {
  /** Distance thresholds in world units */
  thresholds: {
    /** Beyond this distance, use points instead of mesh */
    pointMode: number;
    /** Beyond this distance, don't render at all */
    cullDistance: number;
  };

  /** Point rendering settings */
  points: {
    /** Point size in world units (typically voxelSize) */
    size: number;
    /** Use size attenuation (smaller when far) */
    sizeAttenuation: boolean;
    /** Decimation step for sparse points (1 = all, 2 = every other, etc.) */
    decimationStep: number;
  };

  /** Transition settings */
  transition: {
    /** Hysteresis to prevent flicker at boundary */
    hysteresis: number;
    /** Fade duration in milliseconds (0 = instant) */
    fadeDuration: number;
  };
}

const DEFAULT_LOD_CONFIG: LODConfig = {
  thresholds: {
    pointMode: 256,    // 4 chunks away
    cullDistance: 512, // 8 chunks away
  },
  points: {
    size: 0.1,  // Match voxelSize
    sizeAttenuation: true,
    decimationStep: 2,  // Every other voxel for distant points
  },
  transition: {
    hysteresis: 16,  // Don't switch back until 16 units closer
    fadeDuration: 0, // Instant for now, can add crossfade later
  },
};
```

### Implementation Plan

#### Phase 5.3.1: Point Generation

```rust
/// Generate point cloud from chunk bitmask
#[wasm_bindgen]
pub fn chunk_to_point_cloud(
    opaque_mask: &[u64],
    voxel_size: f32,
    origin: [f32; 3],
    decimation_step: u32,
) -> PointCloudOutput {
    let step = decimation_step as usize;
    let mut positions = Vec::new();
    let mut colors = Vec::new();

    for x in (1..63).step_by(step) {
        for z in (1..63).step_by(step) {
            let column = opaque_mask[x * 64 + z];
            let mut bits = column >> 1;  // Skip padding bit
            let mut y = 1usize;

            while bits != 0 {
                if bits & 1 != 0 && y % step == 0 {
                    positions.push(origin[0] + x as f32 * voxel_size);
                    positions.push(origin[1] + y as f32 * voxel_size);
                    positions.push(origin[2] + z as f32 * voxel_size);

                    // Color from material (simplified for points)
                    colors.extend_from_slice(&[0.5, 0.5, 0.5]); // Gray for now
                }
                bits >>= 1;
                y += 1;
            }
        }
    }

    PointCloudOutput { positions, colors }
}
```

#### Phase 5.3.2: Dual Representation Storage

```typescript
interface ChunkRenderData {
  coord: ChunkCoord;

  // LOD 0: Full mesh (lazy-loaded when close)
  mesh: {
    geometry: THREE.BufferGeometry | null;
    object: THREE.Mesh | null;
    loaded: boolean;
  };

  // LOD 1: Point cloud (always available)
  points: {
    geometry: THREE.BufferGeometry;
    object: THREE.Points;
  };

  // Current state
  activeLOD: 'mesh' | 'points' | 'culled';
  lastDistance: number;
}
```

#### Phase 5.3.3: LOD Transition Logic

```typescript
class ChunkLODManager {
  private config: LODConfig;
  private chunks: Map<string, ChunkRenderData>;

  updateLODs(cameraPosition: THREE.Vector3): void {
    for (const [key, chunk] of this.chunks) {
      const distance = this.getChunkDistance(chunk.coord, cameraPosition);
      const newLOD = this.determineLOD(distance, chunk.activeLOD);

      if (newLOD !== chunk.activeLOD) {
        this.transitionLOD(chunk, newLOD);
      }

      chunk.lastDistance = distance;
    }
  }

  private determineLOD(
    distance: number,
    currentLOD: 'mesh' | 'points' | 'culled'
  ): 'mesh' | 'points' | 'culled' {
    const { thresholds, transition } = this.config;

    // Apply hysteresis to prevent flicker
    const hysteresis = currentLOD === 'mesh' ? 0 : transition.hysteresis;

    if (distance > thresholds.cullDistance) {
      return 'culled';
    } else if (distance > thresholds.pointMode - hysteresis) {
      return 'points';
    } else {
      return 'mesh';
    }
  }

  private transitionLOD(chunk: ChunkRenderData, newLOD: 'mesh' | 'points' | 'culled'): void {
    // Hide current
    if (chunk.mesh.object) chunk.mesh.object.visible = false;
    chunk.points.object.visible = false;

    // Show new
    switch (newLOD) {
      case 'mesh':
        if (!chunk.mesh.loaded) {
          this.loadMesh(chunk);  // Async mesh generation
        }
        if (chunk.mesh.object) {
          chunk.mesh.object.visible = true;
        }
        break;

      case 'points':
        chunk.points.object.visible = true;
        break;

      case 'culled':
        // Both hidden
        break;
    }

    chunk.activeLOD = newLOD;
  }
}
```

### Memory Considerations

| LOD Level | Data per Chunk | Notes |
|-----------|----------------|-------|
| Mesh (close) | ~40-200 KB | Greedy-meshed geometry |
| Points (far) | ~10-50 KB | Position + color only |
| Culled | 0 KB | Not in GPU memory |

For a 16×16×16 chunk world (4096 chunks):
- All mesh: 160-800 MB GPU
- With LOD (25% mesh, 50% points, 25% culled): 40-200 MB GPU

### Future Enhancements

1. **Crossfade transitions**: Render both LODs briefly with alpha blend
2. **Intermediate LOD**: Decimated mesh between full mesh and points
3. **Async mesh loading**: Generate mesh only when chunk enters mesh distance
4. **Point impostor billboards**: Oriented quads instead of points for better appearance

## Consequences

### Positive

- **Leverages existing code**: Point rendering already implemented
- **Simple transition logic**: Binary mesh/points decision
- **Memory efficient**: Points use 1/4 to 1/10 the memory of mesh
- **Fast point generation**: O(n) scan of bitmask, no meshing overhead
- **Configurable**: Distance thresholds easily tuned

### Negative

- **Visual discontinuity**: Points look different from mesh at transition
- **No intermediate LOD**: Jump from full detail to points
- **Point rendering limitations**: No normals, no shadows, no surface detail

### Constraints

- Point size must match voxel size for visual consistency
- Hysteresis required to prevent LOD flicker at boundaries
- Mesh lazy-loading adds complexity to chunk management

## References

- [ADR-0003](0003-binary-greedy-meshing.md) - Binary greedy meshing
- [ADR-0004](0004-chunk-size-64.md) - 64³ chunk size
- [implementation-plan.md](../implementation-plan.md#53-milestone-lod-optional) - Phase 5.3 milestone
- [threejs-buffer-management.md](../threejs-buffer-management.md) - GPU buffer lifecycle
- [Current point rendering](../../apps/web/src/viewer/outputs.ts) - Existing implementation
