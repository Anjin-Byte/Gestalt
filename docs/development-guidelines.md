# Development Guidelines

> **Part of the Voxel Mesh Architecture**
>
> Coding standards and organizational principles for the voxel mesh system.

---

## Core Principles

1. **Single Responsibility**: Each module, struct, and function does one thing well
2. **Explicit over Implicit**: Prefer clear, verbose code over clever shortcuts
3. **Debuggable by Default**: Code structure should make debugging straightforward
4. **No Monoliths**: Split large files before they become unmanageable

---

## File Organization

### Maximum File Size

| Language | Soft Limit | Hard Limit | Action |
|----------|------------|------------|--------|
| Rust | 300 lines | 500 lines | Split into submodules |
| TypeScript | 250 lines | 400 lines | Split into separate files |

*Lines exclude comments and blank lines.*

When approaching limits, split by:
1. Logical grouping (data structures vs algorithms vs I/O)
2. Public API vs internal implementation
3. Feature boundaries

### Rust Module Structure

```
crates/voxel_mesh/
├── Cargo.toml
├── src/
│   ├── lib.rs              # Public API re-exports only
│   ├── voxel/
│   │   ├── mod.rs          # Voxel, VoxelField types
│   │   ├── field.rs        # DenseVoxelField implementation
│   │   └── sparse.rs       # SparseVoxelField (if needed)
│   ├── mesh/
│   │   ├── mod.rs          # MeshOutput, MeshBuilder
│   │   ├── greedy.rs       # Greedy meshing algorithm
│   │   ├── face.rs         # FaceDir, face extraction
│   │   └── quad.rs         # Quad emission logic
│   ├── chunk/
│   │   ├── mod.rs          # Chunk, ChunkCoord
│   │   ├── manager.rs      # ChunkManager
│   │   ├── state.rs        # ChunkState, state machine
│   │   ├── dirty.rs        # DirtyTracker
│   │   └── queue.rs        # RebuildQueue
│   ├── wasm/
│   │   ├── mod.rs          # WASM bindings
│   │   └── transfer.rs     # Array transfer utilities
│   └── debug/
│       ├── mod.rs          # Debug utilities
│       └── stats.rs        # Statistics collection
```

**Rule**: `mod.rs` files should contain only:
- Type definitions (structs, enums)
- Trait definitions
- Re-exports from submodules
- Brief impl blocks (< 50 lines)

**Rule**: Implementation logic goes in dedicated files.

### TypeScript Module Structure

```
apps/web/src/
├── voxel/
│   ├── index.ts            # Public exports
│   ├── types.ts            # Shared type definitions
│   ├── mesh/
│   │   ├── index.ts
│   │   ├── ChunkMeshPool.ts
│   │   ├── DoubleBuffer.ts
│   │   └── GeometryBuilder.ts
│   ├── chunk/
│   │   ├── index.ts
│   │   ├── ChunkManager.ts
│   │   ├── DirtyTracker.ts
│   │   └── RebuildScheduler.ts
│   ├── debug/
│   │   ├── index.ts
│   │   ├── DebugPanel.ts
│   │   ├── ChunkBoundaryView.ts
│   │   ├── ChunkStateView.ts
│   │   └── GreedyMeshView.ts
│   └── wasm/
│       ├── index.ts
│       └── WasmBridge.ts
```

**Rule**: One class per file. File name matches class name.

**Rule**: `index.ts` contains only re-exports:
```typescript
// index.ts
export { ChunkMeshPool } from './ChunkMeshPool';
export { DoubleBuffer } from './DoubleBuffer';
export type { ChunkMeshData } from './types';
```

---

## Function Design

### Maximum Function Length

| Complexity | Max Lines | Notes |
|------------|-----------|-------|
| Simple | 20 | Single operation, no branches |
| Moderate | 40 | Some branching, clear flow |
| Complex | 60 | Must have section comments |

### When to Extract a Function

Extract when ANY of these apply:

1. **Repeated code** (even twice)
2. **Nested depth > 3** (loops/conditionals)
3. **Distinct logical step** (even if used once)
4. **Needs a comment explaining what it does** (the function name becomes the comment)
5. **Could be tested independently**

### Function Extraction Example

**Before** (monolithic):
```rust
pub fn greedy_mesh(field: &DenseVoxelField) -> MeshOutput {
    let mut output = MeshOutput::default();

    for dir in FaceDir::ALL {
        let (axis_u, axis_v, axis_normal) = dir.axes();
        let dims = field.dims;
        let dim_u = dims[axis_u];
        let dim_v = dims[axis_v];
        let dim_normal = dims[axis_normal];

        for slice in 0..dim_normal {
            let mut mask: Vec<u16> = vec![0; (dim_u * dim_v) as usize];

            for v in 0..dim_v {
                for u in 0..dim_u {
                    // ... 50 more lines of mask building ...
                }
            }

            for v in 0..dim_v {
                let mut u = 0;
                while u < dim_u {
                    // ... 80 more lines of greedy merge ...
                }
            }
        }
    }

    output
}
```

**After** (modular):
```rust
pub fn greedy_mesh(field: &DenseVoxelField) -> MeshOutput {
    let mut output = MeshOutput::default();

    for dir in FaceDir::ALL {
        mesh_direction(field, dir, &mut output);
    }

    output
}

fn mesh_direction(field: &DenseVoxelField, dir: FaceDir, output: &mut MeshOutput) {
    let slice_info = SliceInfo::new(field, dir);

    for slice in 0..slice_info.depth {
        let mask = build_face_mask(field, dir, slice, &slice_info);
        merge_and_emit_quads(mask, slice, dir, &slice_info, field, output);
    }
}

fn build_face_mask(
    field: &DenseVoxelField,
    dir: FaceDir,
    slice: u32,
    info: &SliceInfo,
) -> Vec<u16> {
    let mut mask = vec![0u16; info.mask_size()];

    for v in 0..info.dim_v {
        for u in 0..info.dim_u {
            if let Some(material) = get_visible_face(field, dir, u, v, slice, info) {
                mask[info.index(u, v)] = material;
            }
        }
    }

    mask
}

fn get_visible_face(
    field: &DenseVoxelField,
    dir: FaceDir,
    u: u32, v: u32, slice: u32,
    info: &SliceInfo,
) -> Option<u16> {
    let pos = info.to_voxel_pos(u, v, slice);
    let voxel = field.get(pos[0], pos[1], pos[2]);

    if !voxel.solid {
        return None;
    }

    let neighbor = info.neighbor_pos(pos, dir);
    if field.is_solid_signed(neighbor) {
        return None;
    }

    Some(voxel.material_id as u16 + 1)
}

// ... more small, focused functions ...
```

### Function Naming

| Pattern | Use For | Example |
|---------|---------|---------|
| `verb_noun` | Actions | `build_mesh`, `extract_faces` |
| `is_adjective` | Boolean queries | `is_solid`, `is_on_boundary` |
| `get_noun` | Accessors (may compute) | `get_neighbor`, `get_voxel` |
| `noun` | Simple field access | `count`, `len` |
| `try_verb` | Fallible operations | `try_parse`, `try_allocate` |
| `into_noun` | Consuming conversions | `into_mesh`, `into_vec` |
| `as_noun` | Borrowing conversions | `as_slice`, `as_ref` |

---

## Data Structure Design

### Struct Organization

```rust
/// Brief description of what this represents.
///
/// Longer explanation if needed, including:
/// - Invariants that must be maintained
/// - Relationships to other types
/// - Thread safety notes
pub struct ChunkManager {
    // Group 1: Configuration (immutable after construction)
    config: RebuildConfig,
    voxel_size: f32,

    // Group 2: Primary data
    chunks: HashMap<ChunkCoord, Chunk>,

    // Group 3: Derived/cached state
    dirty_tracker: DirtyTracker,
    rebuild_queue: RebuildQueue,

    // Group 4: Statistics/debug (optional)
    #[cfg(debug_assertions)]
    stats: DebugStats,
}
```

### Enum Design

Prefer enums over boolean flags or magic numbers:

```rust
// Bad
pub struct Chunk {
    is_dirty: bool,
    is_meshing: bool,
    is_ready: bool,  // Which combinations are valid??
}

// Good
pub enum ChunkState {
    Clean,
    Dirty,
    Meshing { data_version: u64 },
    ReadyToSwap { data_version: u64 },
}
```

### Type Aliases for Clarity

```rust
// Domain-specific type aliases
pub type VoxelIndex = [u32; 3];
pub type WorldPos = [f32; 3];
pub type MaterialId = u8;

// Function signatures become self-documenting
fn voxel_to_world(index: VoxelIndex, transform: &GridTransform) -> WorldPos;
```

---

## Error Handling

### Rust Error Pattern

```rust
// Define domain-specific errors
#[derive(Debug, thiserror::Error)]
pub enum MeshError {
    #[error("chunk {coord:?} not found")]
    ChunkNotFound { coord: ChunkCoord },

    #[error("buffer allocation failed: requested {requested} bytes")]
    AllocationFailed { requested: usize },

    #[error("invalid voxel field dimensions: {dims:?}")]
    InvalidDimensions { dims: [u32; 3] },
}

// Use Result for fallible operations
pub fn mesh_chunk(&self, coord: ChunkCoord) -> Result<MeshOutput, MeshError> {
    let chunk = self.chunks.get(&coord)
        .ok_or(MeshError::ChunkNotFound { coord })?;

    // ...
}

// Use Option for "not found" that isn't an error
pub fn get_chunk(&self, coord: ChunkCoord) -> Option<&Chunk> {
    self.chunks.get(&coord)
}
```

### TypeScript Error Pattern

```typescript
// Custom error classes
export class ChunkError extends Error {
  constructor(
    message: string,
    public readonly coord: ChunkCoord,
    public readonly code: 'NOT_FOUND' | 'MESH_FAILED' | 'ALLOCATION_FAILED'
  ) {
    super(message);
    this.name = 'ChunkError';
  }
}

// Throw for exceptional cases
function getChunkOrThrow(coord: ChunkCoord): Chunk {
  const chunk = this.chunks.get(chunkKey(coord));
  if (!chunk) {
    throw new ChunkError(
      `Chunk not found: ${coord.x},${coord.y},${coord.z}`,
      coord,
      'NOT_FOUND'
    );
  }
  return chunk;
}

// Return undefined for expected "not found"
function getChunk(coord: ChunkCoord): Chunk | undefined {
  return this.chunks.get(chunkKey(coord));
}
```

---

## Comments and Documentation

### When to Comment

**DO comment:**
- Public API (always)
- Non-obvious invariants
- Performance-critical sections explaining why
- Workarounds for external bugs
- Complex algorithms (with references)

**DON'T comment:**
- What the code does (if it's clear from the code)
- Changelog information (use git)
- Commented-out code (delete it)

### Comment Style

```rust
/// Extracts visible faces from a voxel field using neighbor culling.
///
/// A face is visible if:
/// - The voxel is solid, AND
/// - The neighbor in the face direction is empty (or out of bounds)
///
/// # Performance
/// O(n) where n is the number of voxels. Each voxel checks 6 neighbors.
///
/// # Example
/// ```
/// let faces = extract_visible_faces(&field);
/// assert_eq!(faces.len(), 6); // Single voxel has 6 visible faces
/// ```
pub fn extract_visible_faces(field: &DenseVoxelField) -> Vec<VisibleFace> {
    // ...
}
```

### Section Comments for Long Functions

If a function must exceed 40 lines, use section comments:

```rust
fn complex_operation(&mut self) {
    // === Phase 1: Collect dirty chunks ===
    let dirty = self.collect_dirty_chunks();

    // === Phase 2: Prioritize by camera distance ===
    let prioritized = self.prioritize_chunks(dirty, camera_pos);

    // === Phase 3: Process within budget ===
    for chunk in prioritized.take(self.config.max_per_frame) {
        self.rebuild_chunk(chunk);
    }

    // === Phase 4: Swap completed meshes ===
    self.swap_pending_meshes();
}
```

---

## Testing

### Test File Location

```
# Rust: tests alongside source
crates/voxel_mesh/src/mesh/greedy.rs
crates/voxel_mesh/src/mesh/greedy_tests.rs  # or inline #[cfg(test)] mod

# TypeScript: __tests__ directory
apps/web/src/voxel/mesh/ChunkMeshPool.ts
apps/web/src/voxel/mesh/__tests__/ChunkMeshPool.test.ts
```

### Test Naming

```rust
#[test]
fn single_voxel_produces_six_faces() { }

#[test]
fn adjacent_voxels_share_hidden_face() { }

#[test]
fn boundary_edit_marks_neighbor_dirty() { }

#[test]
fn version_mismatch_discards_stale_mesh() { }
```

Pattern: `{scenario}__{expected_behavior}` or `{action}_{condition}_{result}`

### Test Structure (AAA)

```rust
#[test]
fn greedy_mesh_merges_coplanar_faces() {
    // Arrange
    let mut field = DenseVoxelField::new([10, 1, 10]);
    for x in 0..10 {
        for z in 0..10 {
            field.set(x, 0, z, Voxel::SOLID);
        }
    }

    // Act
    let mesh = greedy_mesh(&field);

    // Assert
    assert_eq!(mesh.triangle_count(), 4); // 2 triangles × 2 faces (top + bottom)
}
```

---

## Code Review Checklist

Before submitting:

- [ ] Files under size limits
- [ ] Functions under length limits
- [ ] No nested depth > 3
- [ ] Public API documented
- [ ] Error cases handled
- [ ] Tests for new functionality
- [ ] No `unwrap()` in library code (use `expect()` or `?`)
- [ ] No `any` types in TypeScript
- [ ] No console.log in production code (use debug flags)

---

## Import Organization

### Rust

```rust
// 1. Standard library
use std::collections::HashMap;

// 2. External crates
use wasm_bindgen::prelude::*;
use thiserror::Error;

// 3. Crate modules (absolute)
use crate::voxel::DenseVoxelField;
use crate::mesh::MeshOutput;

// 4. Local modules (relative)
use super::ChunkState;
use self::queue::RebuildQueue;
```

### TypeScript

```typescript
// 1. Node/external packages
import { BufferGeometry, Mesh } from 'three';

// 2. Internal absolute imports
import { ChunkCoord } from '@/voxel/types';
import { WasmBridge } from '@/voxel/wasm';

// 3. Relative imports
import { ChunkMeshData } from './types';
import { buildGeometry } from './GeometryBuilder';
```

---

## Summary

| Principle | Enforcement |
|-----------|-------------|
| Files < 300-500 lines | Split into modules |
| Functions < 20-60 lines | Extract helper functions |
| Nesting depth ≤ 3 | Early returns, extract functions |
| One class per file | TypeScript convention |
| Public API documented | Required for merge |
| Tests for new code | Required for merge |
| No magic numbers | Use named constants/enums |
| Explicit error handling | Result/Option, no silent failures |
