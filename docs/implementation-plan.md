# Voxel Mesh System - Implementation Plan

> **Part of the Voxel Mesh Architecture**
>
> This document outlines the phased implementation plan with milestones and debug tooling.
>
> Related documents:
> - [Architecture Overview](voxel-mesh-architecture.md)
> - [Greedy Mesh Implementation](greedy-mesh-implementation-plan.md)
> - [Chunk Management System](chunk-management-system.md)
> - [Three.js Buffer Management](threejs-buffer-management.md)

---

## Phase Overview

| Phase | Name | Description | Dependencies |
|-------|------|-------------|--------------|
| 1 | Core Meshing | Binary greedy mesh algorithm (10-50x faster) | None |
| 2 | Chunk System | Multi-chunk storage and dirty tracking | Phase 1 |
| 3 | Render Integration | Three.js buffer management | Phase 1 |
| 4 | Debug Tooling | Visualization and inspection tools | Phases 2, 3 |
| 5 | Optimization | Workers, LOD, memory budget | Phases 2, 3 |
| 6 | Polish | Edge cases, error handling, docs | All |

---

## Phase 1: Core Meshing (Rust) - Binary Algorithm

> Uses bitwise operations to process 64 voxels per instruction.
> See [Binary Greedy Meshing Analysis](binary-greedy-meshing-analysis.md) for algorithm details.

### 1.1 Milestone: Binary Data Structures

**Deliverables:**
- [ ] `BinaryChunk` with opaque_mask (`[u64; 64*64]`) and voxel_types (`[u8; 64³]`)
- [ ] `FaceMasks` for 6 directions (`[u64; 6*64*64]`)
- [ ] `pack_quad()` / `unpack_quad()` for 8-byte quad encoding
- [ ] `MeshOutput` with positions, normals, indices

**Tests:**
- [ ] Bitmask set/get operations
- [ ] Empty chunk produces empty masks
- [ ] Chunk constants: CS_P=64, CS=62 (with padding)

### 1.2 Milestone: Bitwise Face Culling

**Deliverables:**
- [ ] `cull_faces()` using bitwise AND with neighbor columns
- [ ] Y-axis culling via bit shifts (`column & ~(column >> 1)`)
- [ ] X/Z-axis culling via adjacent column comparison
- [ ] Proper handling of 1-voxel padding

**Tests:**
- [ ] Single voxel produces 6 face bits
- [ ] Two adjacent voxels share hidden face
- [ ] Interior voxels produce no face bits
- [ ] Boundary faces handled correctly

### 1.3 Milestone: Binary Greedy Merge

**Deliverables:**
- [ ] `trailing_zeros()` for bit-scanning (find next visible face)
- [ ] Forward-merge tracking arrays
- [ ] Width expansion via consecutive bit checking
- [ ] Height expansion via forward-merge compatibility
- [ ] Separate merge functions for Y/X/Z face axes

**Tests:**
- [ ] 10×10×1 solid slab → 2 quads (not 200)
- [ ] Checkerboard pattern → no merging
- [ ] Different materials → separate quads
- [ ] Determinism: same input → same output
- [ ] Performance: 64³ chunk in <200µs

### 1.4 Milestone: Quad Expansion & WASM

**Deliverables:**
- [ ] `expand_quads_to_mesh()` converting packed quads to vertex arrays
- [ ] Correct winding order per face direction
- [ ] `mesh_voxel_positions()` WASM entry point
- [ ] `mesh_dense_voxels()` WASM entry point
- [ ] Zero-copy typed array transfers where possible

**Tests:**
- [ ] Round-trip: JS → WASM → JS
- [ ] 64³ chunk (262K voxels) meshes without crash
- [ ] Memory freed after result consumed
- [ ] Vertex positions match expected world coordinates

---

## Phase 2: Chunk System (Rust)

> Fixed 64³ chunk size (62³ usable with 1-voxel padding for boundary lookups).

### 2.1 Milestone: Chunk Storage

**Deliverables:**
- [ ] `ChunkCoord` with neighbor calculation
- [ ] `Chunk` struct wrapping `BinaryChunk` (64³ with bitmask + types)
- [ ] `ChunkManager` with HashMap storage
- [ ] World-to-chunk coordinate conversion (64-voxel chunks)

**Tests:**
- [ ] Chunk neighbor calculation
- [ ] Voxel edit updates correct chunk
- [ ] Boundary voxel detection

### 2.2 Milestone: Dirty Tracking

**Deliverables:**
- [ ] `ChunkState` enum (Clean, Dirty, Meshing, ReadyToSwap)
- [ ] `DirtyTracker` with deduped HashSet
- [ ] Boundary neighbor propagation
- [ ] `BoundaryFlags` struct

**Tests:**
- [ ] Interior edit → 1 dirty chunk
- [ ] Boundary edit → 2 dirty chunks
- [ ] Multiple edits to same chunk → 1 entry in queue
- [ ] State transitions are valid

### 2.3 Milestone: Rebuild Queue

**Deliverables:**
- [ ] `RebuildQueue` with priority ordering
- [ ] Camera-distance priority calculation
- [ ] Deduplication (no double-queueing)
- [ ] Frame budget configuration

**Tests:**
- [ ] Closer chunks have higher priority
- [ ] Budget limits chunks per frame
- [ ] Queue drains completely over time

### 2.4 Milestone: Version Consistency

**Deliverables:**
- [ ] `data_version` on chunks
- [ ] Version check before mesh swap
- [ ] Stale mesh discard + re-queue

**Tests:**
- [ ] Edit during meshing → version mismatch → re-queue
- [ ] No edit → version match → swap succeeds

---

## Phase 3: Render Integration (TypeScript)

### 3.1 Milestone: Mesh Pool

**Deliverables:**
- [ ] `ChunkMeshPool` class
- [ ] Stable Mesh object reuse
- [ ] Geometry creation from WASM output
- [ ] Proper disposal

**Tests:**
- [ ] Mesh count stays constant after rebuilds
- [ ] No geometry leaks (check renderer.info)
- [ ] Chunk removal disposes geometry

### 3.2 Milestone: Double Buffering

**Deliverables:**
- [ ] Pending geometry storage
- [ ] Atomic swap operation
- [ ] Old geometry disposal

**Tests:**
- [ ] No visual flicker during swap
- [ ] Memory stable over many rebuilds

### 3.3 Milestone: Clipping Planes

**Deliverables:**
- [ ] `SlicingManager` class
- [ ] X/Y/Z axis slicing
- [ ] Custom plane support
- [ ] Enable/disable toggle

**Tests:**
- [ ] Slicing doesn't trigger geometry rebuild
- [ ] Slice position updates immediately
- [ ] Works with both WebGL and WebGPU

---

## Phase 4: Debug Tooling

### 4.1 Milestone: Greedy Mesh Visualization

**Purpose:** Understand how faces are being merged

**Deliverables:**
- [ ] Quad boundary wireframe overlay
- [ ] Color-coded quads by merge size
- [ ] Per-direction face count display
- [ ] Before/after triangle count comparison

**Implementation:**
```typescript
interface GreedyMeshDebugOptions {
  showQuadBoundaries: boolean;      // Wireframe on merged quad edges
  colorByQuadSize: boolean;         // Heatmap: small=red, large=green
  showFaceDirections: boolean;      // Color faces by normal direction
  showMergeStats: boolean;          // Overlay with merge statistics
}

class GreedyMeshDebugView {
  // Generate debug geometry showing quad boundaries
  generateQuadBoundaryLines(meshData: ChunkMeshData): LineSegments;

  // Generate colored mesh showing merge efficiency
  generateMergeHeatmap(meshData: ChunkMeshData): Mesh;

  // Calculate merge statistics
  calculateMergeStats(meshData: ChunkMeshData): MergeStats;
}

interface MergeStats {
  totalQuads: number;
  avgQuadSize: number;          // In voxel faces
  largestQuad: number;
  mergeEfficiency: number;      // 1.0 = perfect, 0.0 = no merging
  facesByDirection: number[];   // [+X, -X, +Y, -Y, +Z, -Z]
  triangleReduction: number;    // Ratio vs naive
}
```

**Visual:**
```
┌─────────────────────────────────────────┐
│ Greedy Mesh Debug                       │
├─────────────────────────────────────────┤
│ Total Quads: 1,234                      │
│ Avg Quad Size: 8.5 faces                │
│ Largest Quad: 64 faces                  │
│ Merge Efficiency: 87%                   │
│ Triangle Reduction: 12x                 │
│                                         │
│ Faces by Direction:                     │
│   +X: 234  -X: 228                      │
│   +Y: 312  -Y: 298                      │
│   +Z: 189  -Z: 195                      │
└─────────────────────────────────────────┘
```

### 4.2 Milestone: Chunk Boundary Visualization

**Purpose:** See chunk grid and identify which chunk contains what

**Deliverables:**
- [ ] Chunk boundary wireframe grid
- [ ] Chunk coordinate labels
- [ ] Highlight selected chunk
- [ ] Camera-to-chunk distance display

**Implementation:**
```typescript
interface ChunkBoundaryDebugOptions {
  showBoundaries: boolean;          // Wireframe box per chunk
  showLabels: boolean;              // (x,y,z) coordinate labels
  highlightChunk: ChunkCoord | null; // Highlight specific chunk
  showDistances: boolean;           // Distance from camera
  boundaryColor: number;            // Wireframe color
  boundaryOpacity: number;          // 0-1
}

class ChunkBoundaryDebugView {
  private boundaryGroup: Group;
  private labelSprites: Map<string, Sprite>;

  // Update boundary visualization
  update(chunks: Map<string, Chunk>, cameraPos: Vector3): void;

  // Create wireframe box for chunk
  private createChunkWireframe(coord: ChunkCoord): LineSegments;

  // Create text sprite for chunk label
  private createChunkLabel(coord: ChunkCoord): Sprite;
}
```

**Visual:**
```
     ┌───────────┬───────────┐
    /│          /│          /│
   / │   (0,1) / │   (1,1) / │
  ┌───────────┬───────────┐  │
  │  │        │  │        │  │
  │  └────────│──┴────────│──┘
  │ /         │ /         │ /
  │/   (0,0)  │/   (1,0)  │/
  └───────────┴───────────┘

  [Chunk grid with coordinate labels at centers]
```

### 4.3 Milestone: Chunk State Visualization

**Purpose:** See dirty/clean/meshing state at a glance

**Deliverables:**
- [ ] Color-coded chunk overlays by state
- [ ] State transition history log
- [ ] Rebuild queue visualization
- [ ] Real-time state counters

**Implementation:**
```typescript
interface ChunkStateDebugOptions {
  showStateOverlay: boolean;        // Colored overlay per chunk
  showStateLog: boolean;            // Recent state transitions
  showQueueVisualization: boolean;  // Pending rebuild queue
  showCounters: boolean;            // Clean/Dirty/Meshing counts
}

// State colors
const STATE_COLORS = {
  Clean: 0x00ff00,        // Green
  Dirty: 0xff0000,        // Red
  Meshing: 0xffff00,      // Yellow
  ReadyToSwap: 0x00ffff,  // Cyan
};

class ChunkStateDebugView {
  private overlayGroup: Group;
  private stateLog: StateLogEntry[];

  // Update state overlays
  update(chunks: Map<string, Chunk>): void;

  // Create semi-transparent overlay for chunk
  private createStateOverlay(coord: ChunkCoord, state: ChunkState): Mesh;

  // Log state transition
  logTransition(coord: ChunkCoord, from: ChunkState, to: ChunkState): void;

  // Get recent log entries
  getRecentTransitions(count: number): StateLogEntry[];
}

interface StateLogEntry {
  timestamp: number;
  coord: ChunkCoord;
  fromState: ChunkState;
  toState: ChunkState;
}
```

**Visual:**
```
┌─────────────────────────────────────────┐
│ Chunk State Debug                       │
├─────────────────────────────────────────┤
│ ■ Clean: 45    ■ Dirty: 3               │
│ ■ Meshing: 1   ■ ReadyToSwap: 2         │
│                                         │
│ Rebuild Queue: 3 chunks                 │
│   1. (2,0,1) pri=0.95                   │
│   2. (2,0,0) pri=0.82                   │
│   3. (3,0,1) pri=0.45                   │
│                                         │
│ Recent Transitions:                     │
│   12:34:56 (2,0,1) Dirty → Meshing      │
│   12:34:55 (2,0,0) Clean → Dirty        │
│   12:34:55 (2,0,1) Clean → Dirty        │
└─────────────────────────────────────────┘

[3D view shows colored overlays:
 - Green chunks = Clean
 - Red chunks = Dirty
 - Yellow chunks = Meshing
 - Cyan chunks = ReadyToSwap]
```

### 4.4 Milestone: Debug Panel UI

**Purpose:** Unified debug controls and displays

**Deliverables:**
- [ ] Collapsible debug panel
- [ ] Toggle switches for each visualization
- [ ] Real-time statistics
- [ ] Export debug data (JSON)

**Implementation:**
```typescript
interface DebugPanelConfig {
  position: 'top-left' | 'top-right' | 'bottom-left' | 'bottom-right';
  defaultExpanded: boolean;
  sections: DebugSection[];
}

type DebugSection =
  | 'greedy-mesh'
  | 'chunk-boundaries'
  | 'chunk-state'
  | 'performance'
  | 'memory';

class DebugPanel {
  private container: HTMLElement;
  private sections: Map<DebugSection, HTMLElement>;

  // Toggle section visibility
  toggleSection(section: DebugSection): void;

  // Update statistics display
  updateStats(stats: DebugStats): void;

  // Export current debug state
  exportDebugData(): string;
}

interface DebugStats {
  fps: number;
  frameTime: number;
  triangles: number;
  drawCalls: number;
  chunksTotal: number;
  chunksClean: number;
  chunksDirty: number;
  chunksMeshing: number;
  rebuildQueueSize: number;
  memoryUsage: number;
}
```

### 4.5 Debug Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `D` | Toggle debug panel |
| `G` | Toggle greedy mesh visualization |
| `B` | Toggle chunk boundaries |
| `S` | Toggle chunk state overlay |
| `W` | Toggle wireframe mode |
| `N` | Toggle normals visualization |
| `1-6` | Toggle face direction filters (+X/-X/+Y/-Y/+Z/-Z) |

---

## Phase 5: Optimization

### 5.1 Milestone: Web Workers

**Deliverables:**
- [ ] Meshing in dedicated worker
- [ ] Transferable array buffers
- [ ] Job queue management
- [ ] Progress reporting

### 5.2 Milestone: Memory Budget

**Deliverables:**
- [ ] Memory usage tracking
- [ ] LRU chunk eviction
- [ ] Geometry pooling
- [ ] Budget configuration

### 5.3 Milestone: LOD (Optional)

**Deliverables:**
- [ ] Distance-based detail levels
- [ ] Simplified meshes for far chunks
- [ ] Smooth LOD transitions

---

## Phase 6: Polish

### 6.1 Milestone: Error Handling

**Deliverables:**
- [ ] Graceful degradation on mesh failure
- [ ] Memory allocation failure recovery
- [ ] Invalid input validation
- [ ] Error reporting to debug panel

### 6.2 Milestone: Edge Cases

**Deliverables:**
- [ ] Empty chunk handling
- [ ] Single-voxel chunks
- [ ] Maximum chunk size limits
- [ ] Coordinate overflow protection

### 6.3 Milestone: Documentation

**Deliverables:**
- [ ] API documentation
- [ ] Usage examples
- [ ] Performance tuning guide
- [ ] Troubleshooting guide

---

## Testing Strategy

### Test Frameworks

| Language | Framework | Purpose |
|----------|-----------|---------|
| Rust | `cargo test` | Unit tests, doc tests |
| Rust | `criterion` | Benchmarks |
| Rust | `wasm-bindgen-test` | WASM-specific tests |
| TypeScript | `vitest` | Unit + integration tests |
| TypeScript | `playwright` | Visual regression |
| Both | Custom harness | Cross-language validation |

### Test Categories

1. **Unit Tests** - Isolated function/module tests
2. **Integration Tests** - Multi-component interaction
3. **WASM Boundary Tests** - JS ↔ WASM data transfer
4. **Visual Regression Tests** - Rendered output comparison
5. **Performance Benchmarks** - Timing thresholds
6. **Determinism Tests** - Same input → same output

---

### Phase 1 Tests: Core Meshing

#### 1.1 Binary Data Structures

```rust
#[cfg(test)]
mod binary_data_tests {
    use super::*;

    #[test]
    fn bitmask_set_get_roundtrip() {
        let mut mask = OpaqueMask::new();
        mask.set(10, 20, 30, true);
        assert!(mask.get(10, 20, 30));
        assert!(!mask.get(10, 20, 31));
    }

    #[test]
    fn empty_chunk_has_zero_masks() {
        let chunk = BinaryChunk::new();
        for y in 0..64 {
            for z in 0..64 {
                assert_eq!(chunk.opaque_mask[y * 64 + z], 0u64);
            }
        }
    }

    #[test]
    fn chunk_constants_correct() {
        assert_eq!(BinaryChunk::SIZE, 64);
        assert_eq!(BinaryChunk::USABLE_SIZE, 62); // With 1-voxel padding
    }

    #[test]
    fn pack_unpack_quad_roundtrip() {
        let packed = pack_quad(10, 20, 30, 5, 8, 42);
        let (x, y, z, w, h, mat) = unpack_quad(packed);
        assert_eq!((x, y, z, w, h, mat), (10, 20, 30, 5, 8, 42));
    }

    #[test]
    fn pack_quad_max_values() {
        // Test boundary values
        let packed = pack_quad(63, 63, 63, 62, 62, 0xFFFFFFFF);
        let (x, y, z, w, h, mat) = unpack_quad(packed);
        assert_eq!((x, y, z, w, h, mat), (63, 63, 63, 62, 62, 0xFFFFFFFF));
    }
}
```

#### 1.2 Bitwise Face Culling

```rust
#[cfg(test)]
mod face_culling_tests {
    use super::*;

    #[test]
    fn single_voxel_produces_six_faces() {
        let mut chunk = BinaryChunk::new();
        chunk.set_voxel(32, 32, 32, 1); // Center voxel

        let faces = cull_faces(&chunk);

        // Each direction should have exactly 1 face bit set
        for dir in 0..6 {
            assert_eq!(faces[dir].count_ones(), 1);
        }
    }

    #[test]
    fn adjacent_voxels_share_hidden_face() {
        let mut chunk = BinaryChunk::new();
        chunk.set_voxel(32, 32, 32, 1);
        chunk.set_voxel(33, 32, 32, 1); // Adjacent in +X

        let faces = cull_faces(&chunk);

        // Total faces: 12 (6 each) - 2 hidden = 10
        let total: u32 = faces.iter().map(|f| f.count_ones()).sum();
        assert_eq!(total, 10);
    }

    #[test]
    fn interior_voxel_produces_no_faces() {
        let mut chunk = BinaryChunk::new();
        // Create 3x3x3 cube
        for x in 31..34 {
            for y in 31..34 {
                for z in 31..34 {
                    chunk.set_voxel(x, y, z, 1);
                }
        }

        let faces = cull_faces(&chunk);

        // Center voxel (32,32,32) should contribute 0 faces
        // Check that we have 6 faces per exposed surface voxel (26 surface)
        let total: u32 = faces.iter().map(|f| f.count_ones()).sum();
        assert_eq!(total, 54); // 9 faces per side * 6 sides
    }

    #[test]
    fn boundary_faces_at_chunk_edge() {
        let mut chunk = BinaryChunk::new();
        chunk.set_voxel(1, 32, 32, 1); // At -X boundary (index 1 with padding)

        let faces = cull_faces(&chunk);

        // Should have face on -X boundary
        assert!(faces[NEG_X].get(1, 32, 32));
    }
}
```

#### 1.3 Binary Greedy Merge

```rust
#[cfg(test)]
mod greedy_merge_tests {
    use super::*;

    #[test]
    fn solid_slab_merges_to_one_quad() {
        let mut chunk = BinaryChunk::new();
        // 10x10x1 slab on Y=32
        for x in 20..30 {
            for z in 20..30 {
                chunk.set_voxel(x, 32, z, 1);
            }
        }

        let quads = greedy_mesh(&chunk);

        // Should merge to 2 quads: top (+Y) and bottom (-Y)
        assert_eq!(quads.len(), 2);

        // Each quad should cover 10x10 = 100 voxel faces
        for q in &quads {
            let (_, _, _, w, h, _) = unpack_quad(*q);
            assert_eq!(w * h, 100);
        }
    }

    #[test]
    fn checkerboard_no_merging() {
        let mut chunk = BinaryChunk::new();
        // Checkerboard pattern
        for x in 20..30 {
            for y in 20..30 {
                for z in 20..30 {
                    if (x + y + z) % 2 == 0 {
                        chunk.set_voxel(x, y, z, 1);
                    }
                }
            }
        }

        let quads = greedy_mesh(&chunk);

        // Each voxel produces 6 faces, no merging possible
        // 500 voxels * 6 faces = 3000 quads (worst case)
        assert!(quads.len() >= 500); // At least one quad per voxel
    }

    #[test]
    fn different_materials_separate_quads() {
        let mut chunk = BinaryChunk::new();
        // Two adjacent voxels with different materials
        chunk.set_voxel(32, 32, 32, 1);
        chunk.set_voxel(33, 32, 32, 2);

        let quads = greedy_mesh(&chunk);

        // Should not merge due to material difference
        // 10 faces total (6 + 6 - 2 hidden) = 10 quads
        assert_eq!(quads.len(), 10);
    }

    #[test]
    fn deterministic_output() {
        let mut chunk = BinaryChunk::new();
        for x in 20..40 {
            for y in 20..40 {
                for z in 20..40 {
                    if rand_bool(x, y, z) {
                        chunk.set_voxel(x, y, z, 1);
                    }
                }
            }
        }

        let result1 = greedy_mesh(&chunk);
        let result2 = greedy_mesh(&chunk);

        assert_eq!(result1, result2);
    }
}
```

#### 1.4 Quad Expansion & WASM

```rust
#[cfg(test)]
mod expansion_tests {
    use super::*;

    #[test]
    fn expand_produces_correct_vertex_count() {
        let packed = pack_quad(10, 10, 10, 5, 5, 1);
        let output = expand_quad(packed, FACE_POS_Y, 0.1, [0.0, 0.0, 0.0]);

        assert_eq!(output.positions.len(), 12); // 4 vertices * 3 floats
        assert_eq!(output.normals.len(), 12);
        assert_eq!(output.indices.len(), 6); // 2 triangles
    }

    #[test]
    fn expand_correct_winding_order() {
        let packed = pack_quad(0, 0, 0, 1, 1, 1);
        let output = expand_quad(packed, FACE_POS_Y, 1.0, [0.0, 0.0, 0.0]);

        // Check counter-clockwise winding for +Y face
        // Normal should point up (0, 1, 0)
        assert_eq!(output.normals[0..3], [0.0, 1.0, 0.0]);
    }
}

// WASM boundary tests
#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_tests {
    use wasm_bindgen_test::*;
    use super::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn js_roundtrip_positions() {
        let positions = Float32Array::new_with_length(9);
        positions.set_index(0, 1.0);
        positions.set_index(1, 2.0);
        positions.set_index(2, 3.0);
        // ... set all 9 values

        let result = mesh_voxel_positions(&positions, 0.1, 1);

        assert!(result.positions.length() > 0);
    }

    #[wasm_bindgen_test]
    fn large_chunk_no_crash() {
        let mut chunk = BinaryChunk::new();
        // Fill entire chunk
        for x in 1..63 {
            for y in 1..63 {
                for z in 1..63 {
                    chunk.set_voxel(x, y, z, 1);
                }
            }
        }

        let result = mesh_binary_chunk(&chunk);
        assert!(result.positions.length() > 0);
    }
}
```

---

### Phase 2 Tests: Chunk System

```rust
#[cfg(test)]
mod chunk_system_tests {
    use super::*;

    // 2.1 Chunk Storage
    #[test]
    fn chunk_coord_neighbors() {
        let coord = ChunkCoord { x: 5, y: 3, z: 7 };
        let neighbors = coord.neighbors();

        assert!(neighbors.contains(&ChunkCoord { x: 6, y: 3, z: 7 })); // +X
        assert!(neighbors.contains(&ChunkCoord { x: 4, y: 3, z: 7 })); // -X
        assert_eq!(neighbors.len(), 6);
    }

    #[test]
    fn world_to_chunk_coord() {
        // Voxel at world position (70, 32, 200) with CHUNK_SIZE=64
        let world_pos = (70, 32, 200);
        let chunk = ChunkCoord::from_world(world_pos);

        assert_eq!(chunk, ChunkCoord { x: 1, y: 0, z: 3 });
    }

    #[test]
    fn voxel_edit_updates_correct_chunk() {
        let mut manager = ChunkManager::new();
        manager.set_voxel(70, 32, 200, 1);

        let chunk = manager.get_chunk(&ChunkCoord { x: 1, y: 0, z: 3 });
        assert!(chunk.is_some());
        assert!(chunk.unwrap().get_voxel(6, 32, 8).is_solid());
    }

    // 2.2 Dirty Tracking
    #[test]
    fn interior_edit_one_dirty() {
        let mut manager = ChunkManager::new();
        manager.set_voxel(100, 100, 100, 1); // Interior of chunk

        assert_eq!(manager.dirty_count(), 1);
    }

    #[test]
    fn boundary_edit_two_dirty() {
        let mut manager = ChunkManager::new();
        // Edit at X=64 (boundary between chunks 0 and 1)
        manager.set_voxel(64, 32, 32, 1);

        assert_eq!(manager.dirty_count(), 2); // Both neighbors marked dirty
    }

    #[test]
    fn multiple_edits_dedupe() {
        let mut manager = ChunkManager::new();
        manager.set_voxel(10, 10, 10, 1);
        manager.set_voxel(11, 10, 10, 1);
        manager.set_voxel(12, 10, 10, 1);

        // All in same chunk, should be 1 dirty
        assert_eq!(manager.dirty_count(), 1);
    }

    // 2.3 Rebuild Queue
    #[test]
    fn closer_chunks_higher_priority() {
        let mut queue = RebuildQueue::new();
        let camera = Vector3::new(0.0, 0.0, 0.0);

        queue.add(ChunkCoord { x: 10, y: 0, z: 0 }); // Far
        queue.add(ChunkCoord { x: 1, y: 0, z: 0 });  // Close

        queue.sort_by_camera(camera);

        let next = queue.pop();
        assert_eq!(next, Some(ChunkCoord { x: 1, y: 0, z: 0 }));
    }

    #[test]
    fn budget_limits_per_frame() {
        let mut queue = RebuildQueue::new();
        for i in 0..20 {
            queue.add(ChunkCoord { x: i, y: 0, z: 0 });
        }

        let budget = RebuildConfig { max_per_frame: 4 };
        let batch = queue.take_batch(&budget);

        assert_eq!(batch.len(), 4);
        assert_eq!(queue.len(), 16);
    }

    // 2.4 Version Consistency
    #[test]
    fn edit_during_mesh_requeues() {
        let mut manager = ChunkManager::new();
        let coord = ChunkCoord { x: 0, y: 0, z: 0 };

        manager.set_voxel(10, 10, 10, 1);
        let version_before = manager.get_chunk(&coord).unwrap().data_version;

        // Simulate mesh job starting
        manager.start_meshing(coord);

        // Edit while meshing
        manager.set_voxel(11, 10, 10, 2);
        let version_after = manager.get_chunk(&coord).unwrap().data_version;

        assert!(version_after > version_before);

        // Simulate mesh completion with old version
        let result = manager.complete_meshing(coord, version_before, MeshData::default());

        assert_eq!(result, MeshResult::VersionMismatch);
        assert!(manager.is_dirty(&coord)); // Re-queued
    }
}
```

---

### Phase 3 Tests: Render Integration

```typescript
// TypeScript unit tests (vitest)
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { ChunkMeshPool } from './ChunkMeshPool';
import { SlicingManager } from './SlicingManager';

describe('ChunkMeshPool', () => {
  let pool: ChunkMeshPool;
  let scene: THREE.Scene;

  beforeEach(() => {
    scene = new THREE.Scene();
    pool = new ChunkMeshPool(scene);
  });

  afterEach(() => {
    pool.dispose();
  });

  it('reuses mesh objects across rebuilds', () => {
    const coord = { x: 0, y: 0, z: 0 };

    pool.updateChunk(coord, createMockMeshData(100));
    const meshBefore = pool.getMesh(coord);

    pool.updateChunk(coord, createMockMeshData(200));
    const meshAfter = pool.getMesh(coord);

    expect(meshBefore).toBe(meshAfter); // Same object reference
    expect(meshAfter.geometry.attributes.position.count).toBe(200 * 4);
  });

  it('disposes geometry on chunk removal', () => {
    const coord = { x: 0, y: 0, z: 0 };
    pool.updateChunk(coord, createMockMeshData(100));

    const mesh = pool.getMesh(coord);
    const geometry = mesh.geometry;

    pool.removeChunk(coord);

    // Geometry should be disposed (attributes deleted)
    expect(geometry.attributes.position).toBeUndefined();
  });

  it('handles rapid successive updates', async () => {
    const coord = { x: 0, y: 0, z: 0 };

    // Simulate rapid edits
    for (let i = 0; i < 10; i++) {
      pool.updateChunk(coord, createMockMeshData(i * 10 + 50));
    }

    // Only final state should matter
    const mesh = pool.getMesh(coord);
    expect(mesh.geometry.attributes.position.count).toBe(540 * 4);
  });

  it('no geometry leaks after many cycles', () => {
    const coord = { x: 0, y: 0, z: 0 };
    const initialGeometryCount = renderer.info.memory.geometries;

    for (let i = 0; i < 100; i++) {
      pool.updateChunk(coord, createMockMeshData(i));
    }

    const finalGeometryCount = renderer.info.memory.geometries;
    expect(finalGeometryCount).toBeLessThanOrEqual(initialGeometryCount + 1);
  });
});

describe('SlicingManager', () => {
  it('slicing does not trigger geometry rebuild', () => {
    const manager = new SlicingManager();
    const mesh = createMockMesh();
    let rebuildCalled = false;

    manager.onRebuildNeeded = () => { rebuildCalled = true; };
    manager.setSlicePosition('x', 0.5);

    expect(rebuildCalled).toBe(false);
    expect(mesh.material.clippingPlanes.length).toBe(1);
  });

  it('slice position updates immediately', () => {
    const manager = new SlicingManager();
    const material = new THREE.MeshStandardMaterial();

    manager.attachMaterial(material);
    manager.setSlicePosition('x', 0.3);

    const plane = material.clippingPlanes[0];
    expect(plane.constant).toBe(0.3);
  });
});

describe('Double Buffering', () => {
  it('no visual flicker during swap', async () => {
    const buffer = new DoubleBuffer();

    buffer.preparePending(createMockMeshData(100));

    // Active should still show old data
    const activeBefore = buffer.getActive();

    buffer.swap();

    const activeAfter = buffer.getActive();

    // No frame where both are invalid
    expect(activeBefore).toBeDefined();
    expect(activeAfter).toBeDefined();
  });
});
```

---

### WASM Boundary Tests

These tests verify correct data transfer between JavaScript and WASM:

```typescript
// wasm-boundary.test.ts
import { describe, it, expect } from 'vitest';
import { initWasm, meshVoxelPositions, copyMeshResult } from './wasm-adapter';

describe('WASM Boundary', () => {
  it('Float32Array transfers correctly to WASM', async () => {
    await initWasm();

    const positions = new Float32Array([
      0, 0, 0,  // voxel 1
      1, 0, 0,  // voxel 2
      0, 1, 0,  // voxel 3
    ]);

    const result = meshVoxelPositions(positions, 1.0, 1);
    const mesh = copyMeshResult(result);

    // Should produce valid mesh
    expect(mesh.positions.length).toBeGreaterThan(0);
    expect(mesh.positions.length % 3).toBe(0);
  });

  it('copy-on-access prevents memory invalidation', async () => {
    await initWasm();

    const result1 = meshVoxelPositions(createTestPositions(100), 1.0, 1);
    const copied = copyMeshResult(result1);

    // Trigger heap growth
    const result2 = meshVoxelPositions(createTestPositions(10000), 1.0, 1);

    // Copied data should still be valid
    expect(copied.positions[0]).toBeCloseTo(0, 5);
  });

  it('large mesh does not exceed WASM memory', async () => {
    await initWasm();

    // Create positions for ~100K voxels
    const positions = createTestPositions(100000);

    // Should not throw
    const result = meshVoxelPositions(positions, 0.1, 1);
    const mesh = copyMeshResult(result);

    expect(mesh.positions.length).toBeGreaterThan(0);
  });

  it('empty input returns empty mesh', async () => {
    await initWasm();

    const result = meshVoxelPositions(new Float32Array(0), 1.0, 1);
    const mesh = copyMeshResult(result);

    expect(mesh.positions.length).toBe(0);
    expect(mesh.indices.length).toBe(0);
  });
});
```

---

### Visual Regression Tests

```typescript
// visual-regression.test.ts (Playwright)
import { test, expect } from '@playwright/test';

test.describe('Visual Regression', () => {
  test('single chunk renders correctly', async ({ page }) => {
    await page.goto('/test/single-chunk');
    await page.waitForSelector('#render-complete');

    await expect(page).toHaveScreenshot('single-chunk.png', {
      maxDiffPixelRatio: 0.01,
    });
  });

  test('chunk boundaries align', async ({ page }) => {
    await page.goto('/test/multi-chunk-grid');
    await page.waitForSelector('#render-complete');

    // Zoom to boundary
    await page.evaluate(() => camera.position.set(64, 32, 64));
    await page.waitForTimeout(100);

    await expect(page).toHaveScreenshot('chunk-boundary.png', {
      maxDiffPixelRatio: 0.005, // Strict for seams
    });
  });

  test('debug visualization renders', async ({ page }) => {
    await page.goto('/test/debug-view');
    await page.keyboard.press('D'); // Toggle debug panel
    await page.waitForSelector('#debug-panel');

    await expect(page).toHaveScreenshot('debug-panel.png');
  });

  test('greedy mesh visualization', async ({ page }) => {
    await page.goto('/test/greedy-debug');
    await page.keyboard.press('G'); // Toggle greedy viz
    await page.waitForSelector('#greedy-overlay');

    await expect(page).toHaveScreenshot('greedy-merge-viz.png');
  });
});
```

---

### Test Matrix by Milestone

| Milestone | Unit | Integration | WASM | Visual | Benchmark |
|-----------|------|-------------|------|--------|-----------|
| 1.1 Data Structures | ✓ | - | - | - | - |
| 1.2 Face Culling | ✓ | - | - | - | ✓ |
| 1.3 Greedy Merge | ✓ | - | - | - | ✓ |
| 1.4 WASM API | ✓ | ✓ | ✓ | - | ✓ |
| 2.1 Chunk Storage | ✓ | ✓ | - | - | - |
| 2.2 Dirty Tracking | ✓ | ✓ | - | - | - |
| 2.3 Rebuild Queue | ✓ | ✓ | - | - | ✓ |
| 2.4 Version Consistency | ✓ | ✓ | - | - | - |
| 3.1 Mesh Pool | - | ✓ | - | ✓ | - |
| 3.2 Double Buffering | - | ✓ | - | ✓ | - |
| 3.3 Clipping Planes | - | ✓ | - | ✓ | - |
| 4.x Debug Tooling | - | ✓ | - | ✓ | - |

---

### CI Integration

```yaml
# .github/workflows/test.yml
name: Test Suite

on: [push, pull_request]

jobs:
  rust-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable

      - name: Run unit tests
        run: cargo test --lib

      - name: Run WASM tests
        run: |
          rustup target add wasm32-unknown-unknown
          cargo install wasm-pack
          wasm-pack test --headless --chrome

      - name: Run benchmarks (no fail)
        run: cargo bench -- --noplot
        continue-on-error: true

  typescript-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4

      - name: Install dependencies
        run: npm ci

      - name: Run unit tests
        run: npm test

      - name: Run visual regression
        run: npx playwright test
        env:
          CI: true

  benchmark-check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable

      - name: Run benchmarks with thresholds
        run: |
          cargo bench -- --save-baseline ci
          # Compare against main branch baseline
          cargo bench -- --baseline main --threshold 1.1
```

### Visual Regression Tests

- Snapshot comparison of rendered output
- Debug visualization screenshots
- Performance metrics logging

### Performance Benchmarks

Critical benchmarks to run during development and CI:

```rust
#[cfg(test)]
mod benchmarks {
    /// Benchmark: Standard terrain chunk (surface with some caves)
    #[bench]
    fn bench_terrain_chunk_mesh() {
        // Setup: 64³ chunk with ~30% solid voxels, surface-like distribution
        // Target: < 100µs for face culling + greedy merge
    }

    /// Benchmark: Boundary-heavy edit pattern
    /// Tests PaddedChunkView overhead when many neighbors need lookup
    #[bench]
    fn bench_boundary_heavy_rebuild() {
        // Setup: 3x3x3 grid of chunks (27 chunks total)
        // Edit: Modify voxels along all 6 faces of center chunk
        // Measure: Time to rebuild center chunk WITH neighbor lookups
        //
        // For 64³ chunks: 6 × 64² = 24,576 voxels to copy from neighbors
        // Target: < 50µs additional overhead for boundary lookups
        // Alert threshold: > 100µs indicates inefficient PaddedChunkView
    }

    /// Benchmark: Worst-case fragmented chunk
    #[bench]
    fn bench_checkerboard_chunk() {
        // Setup: Alternating solid/empty voxels (checkerboard)
        // This prevents all merging, producing maximum quads
        // Target: < 500µs (worst case is still acceptable)
    }

    /// Benchmark: Multi-material chunk
    /// Tests greedy merge with frequent material boundaries
    #[bench]
    fn bench_multi_material_chunk() {
        // Setup: 64³ chunk with 8 different materials in bands
        // This limits merge opportunities but tests material comparison
        // Target: < 150µs
    }

    /// Benchmark: Empty chunk fast-path
    #[bench]
    fn bench_empty_chunk() {
        // Setup: 64³ chunk with no solid voxels
        // Target: < 10µs (should skip all processing)
    }

    /// Benchmark: Full rebuild queue processing
    #[bench]
    fn bench_rebuild_queue_16_chunks() {
        // Setup: 16 dirty chunks at various distances
        // Measure: Time to process all with priority ordering
        // Target: < 2ms total for frame budget
    }
}
```

```typescript
// TypeScript integration benchmarks
describe('Performance Benchmarks', () => {
  it('boundary-heavy edit completes under threshold', async () => {
    // Setup 3x3x3 chunk grid
    const manager = new ChunkManager();
    setupChunkGrid(manager, 3, 3, 3);

    // Edit boundary voxels of center chunk
    const centerChunk = { x: 1, y: 1, z: 1 };
    const boundaryEdits = generateBoundaryEdits(centerChunk);

    // Time the rebuild including neighbor lookups
    const start = performance.now();
    await manager.setVoxelsBatch(boundaryEdits);
    await manager.processRebuilds();
    const elapsed = performance.now() - start;

    // Alert if boundary lookup overhead is too high
    expect(elapsed).toBeLessThan(5); // 5ms including JS overhead
  });
});
```

---

## Timeline Estimate

| Phase | Duration | Cumulative |
|-------|----------|------------|
| Phase 1: Core Meshing | 1-2 weeks | 2 weeks |
| Phase 2: Chunk System | 1-2 weeks | 4 weeks |
| Phase 3: Render Integration | 1 week | 5 weeks |
| Phase 4: Debug Tooling | 1-2 weeks | 7 weeks |
| Phase 5: Optimization | 1-2 weeks | 9 weeks |
| Phase 6: Polish | 1 week | 10 weeks |

*Note: Phases 2-3 can partially overlap. Debug tooling can be developed incrementally alongside other phases.*

---

## Success Criteria

### Performance
- [ ] 1M voxels mesh in < 100ms (greedy mesh)
- [ ] Chunk rebuild in < 16ms (single chunk)
- [ ] No frame drops during brush painting
- [ ] Memory usage < 500MB for typical scenes

### Correctness
- [ ] Deterministic output
- [ ] No visual seams at chunk boundaries
- [ ] Correct backface culling
- [ ] No geometry leaks

### Usability
- [ ] Debug tools identify issues quickly
- [ ] Clear error messages
- [ ] Smooth edit experience
