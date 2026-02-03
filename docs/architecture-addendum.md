# Architecture Addendum: Addressing Design Gaps

> **Part of the Voxel Mesh Architecture**
>
> This document addresses gaps and ambiguities identified in the initial architecture.
>
> Related documents:
> - [Architecture Overview](voxel-mesh-architecture.md)
> - [Greedy Mesh Implementation](greedy-mesh-implementation-plan.md)
> - [Chunk Management System](chunk-management-system.md)

---

## 1. Cross-Chunk Boundary Meshing

### The Problem

When meshing chunk A, face visibility at the boundary depends on neighboring chunk B's voxels:

```
        Chunk A          │         Chunk B
    ┌─────────────┐      │     ┌─────────────┐
    │             │      │     │             │
    │    [solid]  │ ???  │     │  [empty]    │
    │             │      │     │             │
    └─────────────┘      │     └─────────────┘
                         │
              Should this face be visible?
              (Yes, because B is empty)
```

Without neighbor data, the mesher doesn't know if boundary faces are visible.

### Solution: Padded Chunk Input

The mesher receives a "padded" view that includes a 1-voxel border from neighbors:

```rust
/// A chunk with 1-voxel padding from neighbors for boundary visibility
pub struct PaddedChunkView<'a> {
    /// Core chunk data (CHUNK_SIZE³)
    pub core: &'a Chunk,

    /// Neighbor slices (1 voxel thick each, may be None if no neighbor)
    pub neighbors: NeighborSlices<'a>,
}

pub struct NeighborSlices<'a> {
    pub pos_x: Option<&'a [Voxel]>,  // CHUNK_SIZE² voxels
    pub neg_x: Option<&'a [Voxel]>,
    pub pos_y: Option<&'a [Voxel]>,
    pub neg_y: Option<&'a [Voxel]>,
    pub pos_z: Option<&'a [Voxel]>,
    pub neg_z: Option<&'a [Voxel]>,
}

impl<'a> PaddedChunkView<'a> {
    /// Get voxel at position, including 1-voxel border
    /// Coordinates: -1 to CHUNK_SIZE (inclusive)
    pub fn get(&self, x: i32, y: i32, z: i32) -> Voxel {
        // Check if in core chunk
        if x >= 0 && x < CHUNK_SIZE as i32
            && y >= 0 && y < CHUNK_SIZE as i32
            && z >= 0 && z < CHUNK_SIZE as i32
        {
            return self.core.get(x as u32, y as u32, z as u32);
        }

        // Check neighbor slices
        if x == -1 {
            return self.get_from_neighbor(self.neighbors.neg_x, y, z);
        }
        if x == CHUNK_SIZE as i32 {
            return self.get_from_neighbor(self.neighbors.pos_x, y, z);
        }
        // ... similar for y, z

        // Outside padded region = empty
        Voxel::EMPTY
    }

    fn get_from_neighbor(&self, slice: Option<&[Voxel]>, a: i32, b: i32) -> Voxel {
        match slice {
            Some(data) => {
                if a >= 0 && a < CHUNK_SIZE as i32 && b >= 0 && b < CHUNK_SIZE as i32 {
                    data[(b * CHUNK_SIZE as i32 + a) as usize]
                } else {
                    Voxel::EMPTY
                }
            }
            None => Voxel::EMPTY, // No neighbor = treat as empty (chunk edge)
        }
    }
}
```

### Mesher Integration

```rust
impl ChunkManager {
    /// Build padded view for meshing
    fn build_padded_view(&self, coord: ChunkCoord) -> Option<PaddedChunkView> {
        let core = self.chunks.get(&coord)?;

        let neighbors = NeighborSlices {
            pos_x: self.get_boundary_slice(coord, FaceDir::PosX),
            neg_x: self.get_boundary_slice(coord, FaceDir::NegX),
            pos_y: self.get_boundary_slice(coord, FaceDir::PosY),
            neg_y: self.get_boundary_slice(coord, FaceDir::NegY),
            pos_z: self.get_boundary_slice(coord, FaceDir::PosZ),
            neg_z: self.get_boundary_slice(coord, FaceDir::NegZ),
        };

        Some(PaddedChunkView { core, neighbors })
    }

    /// Extract the boundary slice from a neighbor chunk
    fn get_boundary_slice(&self, coord: ChunkCoord, dir: FaceDir) -> Option<&[Voxel]> {
        let neighbor_coord = coord.neighbor(dir);
        let neighbor = self.chunks.get(&neighbor_coord)?;

        // Return the slice of voxels on the face touching our chunk
        Some(neighbor.get_face_slice(dir.opposite()))
    }
}
```

### Why Not Query On-Demand?

Querying neighbors during meshing adds complexity:
- Mesher needs reference to ChunkManager (coupling)
- HashMap lookups in hot loop (performance)
- Thread safety concerns if meshing in workers

Padding upfront is simpler and enables parallel meshing.

---

## 2. Voxelizer → Chunk System Bridge

### Current Output Format

The existing voxelizer produces:
```typescript
interface VoxelizerOutput {
  positions: Float32Array;  // [x,y,z, x,y,z, ...] world coordinates
  color?: [number, number, number];  // Single color for all voxels
  voxelSize: number;
}
```

### Required Input Format

The chunk system expects:
```typescript
interface ChunkInput {
  coord: ChunkCoord;
  voxels: Uint8Array;  // Packed occupancy + material per voxel
}
```

### Conversion Pipeline

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│   Voxelizer  │────▶│  Converter   │────▶│    Chunk     │────▶│   Greedy     │
│  (positions) │     │ (grid snap)  │     │   Storage    │     │   Mesher     │
└──────────────┘     └──────────────┘     └──────────────┘     └──────────────┘
```

### Converter Implementation

```rust
/// Convert position list to chunked voxel grid
pub fn positions_to_chunks(
    positions: &[f32],      // [x,y,z, x,y,z, ...]
    voxel_size: f32,
    material_id: u8,        // Default material for all voxels
) -> HashMap<ChunkCoord, Chunk> {
    let mut chunks: HashMap<ChunkCoord, Chunk> = HashMap::new();

    let voxel_count = positions.len() / 3;
    for i in 0..voxel_count {
        let world_pos = [
            positions[i * 3],
            positions[i * 3 + 1],
            positions[i * 3 + 2],
        ];

        // Snap to voxel grid
        let voxel_idx = world_to_voxel_index(world_pos, voxel_size);

        // Determine chunk
        let chunk_coord = ChunkCoord::from_voxel(voxel_idx, CHUNK_SIZE);
        let local = voxel_to_local(voxel_idx, CHUNK_SIZE);

        // Get or create chunk
        let chunk = chunks
            .entry(chunk_coord)
            .or_insert_with(|| Chunk::new(chunk_coord));

        // Set voxel
        chunk.set_voxel(
            local[0], local[1], local[2],
            Voxel { solid: true, material_id }
        );
    }

    chunks
}

fn world_to_voxel_index(world_pos: [f32; 3], voxel_size: f32) -> [i32; 3] {
    [
        (world_pos[0] / voxel_size).floor() as i32,
        (world_pos[1] / voxel_size).floor() as i32,
        (world_pos[2] / voxel_size).floor() as i32,
    ]
}

fn voxel_to_local(voxel: [i32; 3], chunk_size: u32) -> [u32; 3] {
    let cs = chunk_size as i32;
    [
        voxel[0].rem_euclid(cs) as u32,
        voxel[1].rem_euclid(cs) as u32,
        voxel[2].rem_euclid(cs) as u32,
    ]
}
```

### WASM API

```rust
#[wasm_bindgen]
pub fn convert_positions_to_chunks(
    positions: &[f32],
    voxel_size: f32,
    material_id: u8,
) -> ChunkCollection {
    let chunks = positions_to_chunks(positions, voxel_size, material_id);
    ChunkCollection::new(chunks)
}
```

### Alternative: Direct Voxelizer Integration

For better performance, modify the voxelizer to output directly to chunks:

```rust
// In voxelizer
pub fn voxelize_to_chunks(
    mesh: &TriangleMesh,
    voxel_size: f32,
    chunk_size: u32,
) -> HashMap<ChunkCoord, Chunk> {
    // During voxelization, write directly to chunk storage
    // instead of collecting positions
}
```

This avoids the intermediate position array and conversion step.

---

## 3. Material/Color Strategy

### The Core Question

How do per-voxel materials interact with greedy meshing?

**Answer**: Greedy meshing only merges faces with **identical materials**. Different materials = no merge.

### Visual Example

```
Single material (merges well):       Multiple materials (no merge):
┌─────────────────────┐              ┌──┬──┬──┬──┬──┐
│                     │              │R │B │R │B │R │
│    1 merged quad    │              ├──┼──┼──┼──┼──┤
│    (2 triangles)    │              │B │R │B │R │B │
│                     │              ├──┼──┼──┼──┼──┤
└─────────────────────┘              │R │B │R │B │R │
                                     └──┴──┴──┴──┴──┘
                                     25 separate quads
                                     (50 triangles)
```

### Material Definition

```rust
/// Material identifier (0-255)
/// 0 = default material
/// 1-255 = custom materials
pub type MaterialId = u8;

/// Material properties (stored separately, not per-voxel)
pub struct MaterialDef {
    pub id: MaterialId,
    pub color: [f32; 3],      // RGB 0.0-1.0
    pub roughness: f32,       // 0.0-1.0
    pub metalness: f32,       // 0.0-1.0
    pub emissive: [f32; 3],   // RGB emission
}

/// Material palette (shared across all chunks)
pub struct MaterialPalette {
    materials: [MaterialDef; 256],
}
```

### Greedy Mesh Mask with Materials

The mask stores `material_id + 1` (0 = no face):

```rust
fn build_face_mask(
    view: &PaddedChunkView,
    dir: FaceDir,
    slice: u32,
    info: &SliceInfo,
) -> Vec<u16> {
    let mut mask = vec![0u16; info.mask_size()];

    for v in 0..info.dim_v {
        for u in 0..info.dim_u {
            let pos = info.to_voxel_pos(u, v, slice);
            let voxel = view.get(pos[0], pos[1], pos[2]);

            if !voxel.solid {
                continue;
            }

            // Check neighbor visibility
            let neighbor_pos = info.neighbor_pos(pos, dir);
            if view.get(neighbor_pos[0], neighbor_pos[1], neighbor_pos[2]).solid {
                continue; // Hidden face
            }

            // Store material_id + 1 (so 0 means "no face")
            mask[info.index(u, v)] = voxel.material_id as u16 + 1;
        }
    }

    mask
}
```

### Greedy Merge Respects Materials

```rust
fn greedy_merge_slice(mask: &mut [u16], ...) {
    for v in 0..dim_v {
        let mut u = 0;
        while u < dim_u {
            let material = mask[(v * dim_u + u) as usize];
            if material == 0 {
                u += 1;
                continue;
            }

            // Expand width - ONLY matching materials
            let mut width = 1u32;
            while u + width < dim_u {
                if mask[(v * dim_u + u + width) as usize] != material {
                    break;  // Different material, stop expanding
                }
                width += 1;
            }

            // Expand height - ONLY matching materials
            let mut height = 1u32;
            'height: while v + height < dim_v {
                for du in 0..width {
                    if mask[((v + height) * dim_u + u + du) as usize] != material {
                        break 'height;  // Different material, stop
                    }
                }
                height += 1;
            }

            // Emit quad with this material
            let actual_material = (material - 1) as u8;
            emit_quad_with_material(u, v, slice, width, height, actual_material, ...);

            // Clear merged region
            // ...
        }
    }
}
```

### Three.js Rendering Options

#### Option A: Vertex Colors (Recommended)

Store color per-vertex. Single material with `vertexColors: true`.

```rust
// During quad emission
fn emit_quad_with_material(
    ...,
    material_id: u8,
    palette: &MaterialPalette,
    output: &mut MeshOutput,
) {
    let mat = palette.get(material_id);
    let color = mat.color;

    // Add 4 vertices for quad
    for corner in &corners {
        output.positions.extend_from_slice(corner);
        output.normals.extend_from_slice(&normal);

        // Vertex color from material palette
        output.colors.extend_from_slice(&color);
    }
}
```

```typescript
// Three.js
const material = new MeshStandardMaterial({
  vertexColors: true,  // Use per-vertex colors
  roughness: 0.35,
  metalness: 0.1,
});
```

**Pros:**
- Single draw call per chunk
- Simple implementation
- Efficient GPU usage

**Cons:**
- All quads share same roughness/metalness
- No per-material properties beyond color

#### Option B: Material Groups (Multi-Material)

Group triangles by material. Use material array on Mesh.

```rust
// During meshing, track material groups
pub struct MeshOutput {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub indices: Vec<u32>,
    pub groups: Vec<MaterialGroup>,  // NEW
}

pub struct MaterialGroup {
    pub start: u32,       // First index
    pub count: u32,       // Number of indices
    pub material_id: u8,
}
```

##### Material Sorting Algorithm

To enable material groups, quads must be sorted by material during emission. This happens after greedy merging:

```rust
/// Sort packed quads by material for draw call batching
fn sort_quads_by_material(packed_quads: &mut [Vec<u64>; 6]) {
    for quads in packed_quads.iter_mut() {
        // Sort by material ID (bits 32-63 of packed quad)
        quads.sort_unstable_by_key(|quad| (quad >> 32) as u32);
    }
}

/// Emit quads as material groups during expansion
fn expand_quads_to_grouped_mesh(
    packed_quads: &[Vec<u64>; 6],
    voxel_size: f32,
    origin: [f32; 3],
) -> MeshOutput {
    let mut output = MeshOutput::default();
    let mut current_material: Option<u32> = None;
    let mut group_start: u32 = 0;
    let mut group_count: u32 = 0;

    for (face, quads) in packed_quads.iter().enumerate() {
        for &quad in quads {
            let (x, y, z, w, h, material) = unpack_quad(quad);

            // Check for material transition
            if Some(material) != current_material {
                // Emit previous group if exists
                if let Some(mat_id) = current_material {
                    if group_count > 0 {
                        output.groups.push(MaterialGroup {
                            start: group_start,
                            count: group_count,
                            material_id: mat_id as u8,
                        });
                    }
                }
                // Start new group
                current_material = Some(material);
                group_start = output.indices.len() as u32;
                group_count = 0;
            }

            // Emit quad vertices and indices
            emit_expanded_quad(face, x, y, z, w, h, voxel_size, origin, &mut output);
            group_count += 6; // 6 indices per quad (2 triangles)
        }
    }

    // Emit final group
    if let Some(mat_id) = current_material {
        if group_count > 0 {
            output.groups.push(MaterialGroup {
                start: group_start,
                count: group_count,
                material_id: mat_id as u8,
            });
        }
    }

    output
}
```

##### Optimization: Skip Sorting for Single-Material Chunks

```rust
fn should_sort_materials(packed_quads: &[Vec<u64>; 6]) -> bool {
    let mut seen_material: Option<u32> = None;

    for quads in packed_quads.iter() {
        for &quad in quads {
            let material = (quad >> 32) as u32;
            match seen_material {
                None => seen_material = Some(material),
                Some(m) if m != material => return true,  // Multiple materials
                _ => {}
            }
        }
    }

    false  // Single material, no sorting needed
}

fn mesh_chunk_with_groups(chunk: &BinaryChunk, ...) -> MeshOutput {
    // ... face culling and greedy merge ...

    // Only sort if multiple materials present
    if should_sort_materials(&packed_quads) {
        sort_quads_by_material(&mut packed_quads);
    }

    expand_quads_to_grouped_mesh(&packed_quads, voxel_size, origin)
}
```

```typescript
// Three.js
const materials = palette.map(mat => new MeshStandardMaterial({
  color: new Color(mat.color[0], mat.color[1], mat.color[2]),
  roughness: mat.roughness,
  metalness: mat.metalness,
}));

const mesh = new Mesh(geometry, materials);

// Apply groups
for (const group of meshData.groups) {
  geometry.addGroup(group.start, group.count, group.materialId);
}
```

**Pros:**
- Per-material properties (roughness, metalness, etc.)
- Different shaders per material possible
- Skip sorting optimization minimizes overhead for simple chunks

**Cons:**
- Multiple draw calls per chunk (one per material used)
- O(n log n) sorting overhead for multi-material chunks
- Higher GPU overhead

#### Option C: Texture Atlas / Palette Texture

Encode material as UV coordinates into a 256x1 palette texture.

```rust
// Store material as UV.x (0-255 maps to 0.0-1.0)
fn emit_quad_with_material(..., material_id: u8, output: &mut MeshOutput) {
    let u_coord = (material_id as f32 + 0.5) / 256.0;

    for corner in &corners {
        output.positions.extend_from_slice(corner);
        output.normals.extend_from_slice(&normal);
        output.uvs.extend_from_slice(&[u_coord, 0.5]);  // Sample center of texel
    }
}
```

```typescript
// Create palette texture
const paletteData = new Uint8Array(256 * 4);  // RGBA
for (let i = 0; i < palette.length; i++) {
  paletteData[i * 4 + 0] = palette[i].color[0] * 255;
  paletteData[i * 4 + 1] = palette[i].color[1] * 255;
  paletteData[i * 4 + 2] = palette[i].color[2] * 255;
  paletteData[i * 4 + 3] = 255;
}

const paletteTexture = new DataTexture(paletteData, 256, 1);
paletteTexture.needsUpdate = true;
paletteTexture.minFilter = NearestFilter;
paletteTexture.magFilter = NearestFilter;

const material = new MeshStandardMaterial({
  map: paletteTexture,
});
```

**Pros:**
- Single draw call
- Easy to update palette without remeshing
- Can encode additional data in texture (roughness in G, metalness in B)

**Cons:**
- Requires UV coordinates per vertex
- Slightly more complex shader setup

### Recommendation

**Start with Option A (Vertex Colors)** for simplicity. It handles per-voxel colors efficiently and is sufficient for most use cases.

If you need per-material roughness/metalness later, migrate to Option C (Palette Texture) which maintains single-draw-call efficiency.

### Editing Materials

When user edits a voxel's material:

```rust
impl ChunkManager {
    pub fn set_voxel_material(
        &mut self,
        world_pos: [f32; 3],
        material_id: MaterialId,
    ) {
        let voxel_idx = self.world_to_voxel(world_pos);
        let chunk_coord = ChunkCoord::from_voxel(voxel_idx, CHUNK_SIZE);

        if let Some(chunk) = self.chunks.get_mut(&chunk_coord) {
            let local = self.voxel_to_local(voxel_idx);
            let mut voxel = chunk.get_voxel(local[0], local[1], local[2]);

            if voxel.solid {
                voxel.material_id = material_id;
                chunk.set_voxel(local[0], local[1], local[2], voxel);

                // Mark dirty - material change requires remesh
                self.dirty_tracker.mark_dirty(chunk_coord);
            }
        }
    }
}
```

**Important**: Material edits require remeshing because greedy merge boundaries may change.

---

## 4. Migration Path

### Current System Analysis

The existing codebase has a clean, modular architecture:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         CURRENT ARCHITECTURE                            │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  modules/wasmVoxelizer.ts                                               │
│    ├─ UI: params (gridDim, voxelSize, renderMode)                      │
│    ├─ Loads OBJ models                                                  │
│    ├─ Calls VoxelizerAdapter → WASM                                    │
│    └─ Emits: ModuleOutput{ kind: "voxels", voxels: VoxelsDescriptor }  │
│                                                                         │
│  modules/types.ts                                                       │
│    └─ VoxelsDescriptor { positions, voxelSize, renderMode, color, ... }│
│                                                                         │
│  viewer/outputs.ts                                                      │
│    └─ buildVoxels() → Points or InstancedMesh                          │
│                                                                         │
│  packages/voxelizer-js/                                                 │
│    └─ VoxelizerAdapter wraps WASM calls                                │
│                                                                         │
│  crates/wasm_voxelizer/                                                 │
│    └─ GpuVoxelizer: sparse brick-based voxelization                    │
│                                                                         │
│  crates/voxelizer/                                                      │
│    └─ Core Rust voxelization (GPU + CPU fallback)                      │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

**Key Abstractions to Preserve:**
1. `TestbedModule` interface - modules return `ModuleOutput[]`
2. `ModuleOutput` union type - standardized output contract
3. `VoxelizerAdapter` - high-level WASM wrapper
4. Separation: computation (modules) ↔ visualization (viewer)

### Migration Strategy: Extend, Don't Replace

Rather than replacing the existing system, we **extend** it:

1. Add new `renderMode: "greedy"` to `VoxelsDescriptor`
2. Add new output kind `"voxelMesh"` for chunk-based geometry
3. Add rendering path in `outputs.ts`
4. New WASM functions in existing `wasm_voxelizer` crate

This preserves backwards compatibility and allows gradual adoption.

### Detailed Migration Plan

#### Step 1: Extend Type Definitions

```typescript
// modules/types.ts - ADD to existing VoxelsDescriptor

export type VoxelRenderMode = "points" | "cubes" | "greedy";

export type VoxelsDescriptor = {
  positions: Float32Array;
  voxelSize: number;
  renderMode: VoxelRenderMode;  // Extended
  color?: Vec3Tuple;
  chunkSize?: number;
  // ... existing fields
};

// ADD new output kind for pre-built mesh geometry
export type VoxelMeshDescriptor = {
  positions: Float32Array;
  normals: Float32Array;
  indices: Uint32Array;
  colors?: Float32Array;
  chunkCoord?: [number, number, number];
};

// Extend ModuleOutput union
export type ModuleOutput =
  | { kind: "mesh"; mesh: MeshDescriptor; label?: string }
  | { kind: "voxels"; voxels: VoxelsDescriptor; label?: string }
  | { kind: "voxelMesh"; voxelMesh: VoxelMeshDescriptor; label?: string }  // NEW
  | { kind: "lines"; lines: LinesDescriptor; label?: string }
  | { kind: "points"; points: PointsDescriptor; label?: string }
  | { kind: "texture2d"; texture: TextureDescriptor; label?: string };
```

#### Step 2: Extend WASM Voxelizer (Rust)

Add greedy meshing to the existing `wasm_voxelizer` crate:

```
crates/wasm_voxelizer/
├── src/
│   ├── lib.rs              # Add new entry points
│   └── meshing/            # NEW: Greedy meshing module
│       ├── mod.rs
│       ├── greedy.rs       # Greedy mesh algorithm
│       ├── face.rs         # Face extraction
│       └── chunk.rs        # Chunk handling
```

```rust
// crates/wasm_voxelizer/src/lib.rs - ADD new methods

#[wasm_bindgen]
impl WasmVoxelizer {
    // Existing methods unchanged...

    /// Voxelize and mesh in one step (for static models)
    #[wasm_bindgen]
    pub async fn voxelize_and_mesh(
        &self,
        positions: &[f32],
        indices: &[u32],
        grid: &JsValue,
    ) -> Result<JsValue, JsValue> {
        // 1. Voxelize (existing code)
        // 2. Convert positions → chunk grid
        // 3. Greedy mesh each chunk
        // 4. Return mesh geometry
    }

    /// Mesh existing voxel positions (for already-voxelized data)
    #[wasm_bindgen]
    pub fn mesh_voxel_positions(
        &self,
        positions: &[f32],
        voxel_size: f32,
        material_id: u8,
    ) -> Result<JsValue, JsValue> {
        // Convert positions → chunks → greedy mesh
    }
}
```

#### Step 3: Extend VoxelizerAdapter (TypeScript)

```typescript
// packages/voxelizer-js/index.ts - ADD new methods

export class VoxelizerAdapter {
  // Existing methods unchanged...

  /**
   * Voxelize and return greedy-meshed geometry
   */
  async voxelizeAndMesh(
    positions: Float32Array,
    indices: Uint32Array,
    gridSpec: VoxelGridSpec,
  ): Promise<VoxelMeshOutput> {
    const result = await this.wasm.voxelize_and_mesh(
      positions,
      indices,
      gridSpec,
    );
    return {
      positions: new Float32Array(result.positions),
      normals: new Float32Array(result.normals),
      indices: new Uint32Array(result.indices),
      colors: result.colors ? new Float32Array(result.colors) : undefined,
      triangleCount: result.triangle_count,
      vertexCount: result.vertex_count,
    };
  }

  /**
   * Mesh existing voxel positions with greedy algorithm
   */
  meshVoxelPositions(
    positions: Float32Array,
    voxelSize: number,
    materialId: number = 0,
  ): VoxelMeshOutput {
    return this.wasm.mesh_voxel_positions(positions, voxelSize, materialId);
  }
}
```

#### Step 4: Update wasmVoxelizer Module

```typescript
// modules/wasmVoxelizer.ts - ADD greedy option to UI and run()

// In createParams():
ui.addDropdown("renderMode", "Render Mode", "greedy", [
  { value: "points", label: "Points" },
  { value: "cubes", label: "Cubes (Instanced)" },
  { value: "greedy", label: "Greedy Mesh" },  // NEW
]);

// In run():
if (params.renderMode === "greedy") {
  // Option A: Voxelize + mesh in one WASM call
  const meshResult = await adapter.voxelizeAndMesh(positions, indices, gridSpec);
  emitOutputs([{
    kind: "voxelMesh",
    voxelMesh: meshResult,
    label: "voxels-greedy",
  }]);
} else {
  // Existing path for points/cubes
  const voxelResult = await adapter.voxelizePositions(...);
  emitOutputs([{
    kind: "voxels",
    voxels: { ...voxelResult, renderMode: params.renderMode },
  }]);
}
```

#### Step 5: Add Rendering Path

```typescript
// viewer/outputs.ts - ADD new builder

const buildVoxelMesh = (output: Extract<ModuleOutput, { kind: "voxelMesh" }>) => {
  const { voxelMesh } = output;
  const geometry = new BufferGeometry();

  geometry.setAttribute("position", new BufferAttribute(voxelMesh.positions, 3));
  geometry.setAttribute("normal", new BufferAttribute(voxelMesh.normals, 3));
  geometry.setIndex(new BufferAttribute(voxelMesh.indices, 1));

  if (voxelMesh.colors) {
    geometry.setAttribute("color", new BufferAttribute(voxelMesh.colors, 3));
  }

  geometry.computeBoundingBox();
  geometry.computeBoundingSphere();

  const material = new MeshStandardMaterial({
    color: 0x7ad8ff,
    roughness: 0.35,
    metalness: 0.1,
    vertexColors: Boolean(voxelMesh.colors),
    side: DoubleSide,
  });

  const mesh = new Mesh(geometry, material);
  mesh.name = output.label ?? "voxel-mesh";
  return mesh;
};

// In buildOutputObject():
export const buildOutputObject = (output: ModuleOutput) => {
  switch (output.kind) {
    case "mesh":
      return { object: buildMesh(output) };
    case "voxels":
      return { object: buildVoxels(output) };  // Existing
    case "voxelMesh":
      return { object: buildVoxelMesh(output) };  // NEW
    case "lines":
      return { object: buildLines(output) };
    // ...
  }
};
```

### File Changes Summary

```
MODIFY (minimal changes):
  modules/types.ts           - Add VoxelMeshDescriptor, extend ModuleOutput
  modules/wasmVoxelizer.ts   - Add "greedy" render mode option
  viewer/outputs.ts          - Add buildVoxelMesh()
  packages/voxelizer-js/     - Add meshVoxelPositions(), voxelizeAndMesh()

ADD (new files in existing crate):
  crates/wasm_voxelizer/src/meshing/
    mod.rs
    greedy.rs
    face.rs
    chunk.rs

NO CHANGES (preserved):
  modules/moduleHost.ts      - Module system unchanged
  viewer/Viewer.ts           - Output handling unchanged
  viewer/threeBackend.ts     - Three.js setup unchanged
  crates/voxelizer/          - Core voxelizer unchanged
```

### Migration Phases

#### Phase 1: Core Implementation (Rust)
- Add `meshing/` module to `wasm_voxelizer`
- Implement greedy meshing algorithm
- Add WASM bindings

#### Phase 2: TypeScript Integration
- Extend types in `modules/types.ts`
- Add methods to `VoxelizerAdapter`
- Add "greedy" option in `wasmVoxelizer.ts`
- Add `buildVoxelMesh()` in `outputs.ts`

#### Phase 3: Testing & Validation
- Compare visual output: points vs greedy
- Performance benchmarks
- Memory usage comparison

#### Phase 4: Chunk System (Optional, Later)
If editing support is needed:
- Add chunk management in TypeScript
- Dirty tracking for incremental updates
- Double-buffered geometry swaps

### Backwards Compatibility

- Existing modules continue to work unchanged
- Existing "points" and "cubes" modes preserved
- New "greedy" mode is opt-in
- No breaking changes to `ModuleOutput` contract

---

## 5. WASM API Design

### Unified API

```rust
// lib.rs - Public WASM API

/// Convert raw positions to chunked storage and mesh all chunks
#[wasm_bindgen]
pub fn mesh_from_positions(
    positions: &[f32],
    voxel_size: f32,
    material_id: u8,
) -> MeshResult {
    let chunks = positions_to_chunks(positions, voxel_size, material_id);
    mesh_all_chunks(&chunks)
}

/// Mesh a single chunk (for incremental updates)
#[wasm_bindgen]
pub fn mesh_chunk(
    chunk_data: &[u8],          // Packed voxel data
    neighbor_data: &[u8],       // Packed neighbor slices (optional)
    voxel_size: f32,
) -> MeshResult {
    let chunk = Chunk::from_packed(chunk_data);
    let neighbors = NeighborSlices::from_packed(neighbor_data);
    let view = PaddedChunkView { core: &chunk, neighbors };
    greedy_mesh(&view, voxel_size)
}

/// Result containing mesh data
#[wasm_bindgen]
pub struct MeshResult {
    positions: Vec<f32>,
    normals: Vec<f32>,
    indices: Vec<u32>,
    colors: Vec<f32>,
}

#[wasm_bindgen]
impl MeshResult {
    // Getters that return references to avoid copying
    #[wasm_bindgen(getter)]
    pub fn positions(&self) -> js_sys::Float32Array {
        unsafe { js_sys::Float32Array::view(&self.positions) }
    }

    #[wasm_bindgen(getter)]
    pub fn normals(&self) -> js_sys::Float32Array {
        unsafe { js_sys::Float32Array::view(&self.normals) }
    }

    #[wasm_bindgen(getter)]
    pub fn indices(&self) -> js_sys::Uint32Array {
        unsafe { js_sys::Uint32Array::view(&self.indices) }
    }

    #[wasm_bindgen(getter)]
    pub fn colors(&self) -> js_sys::Float32Array {
        unsafe { js_sys::Float32Array::view(&self.colors) }
    }

    #[wasm_bindgen(getter)]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    #[wasm_bindgen(getter)]
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }
}
```

### TypeScript Interface

```typescript
// wasm/types.ts
export interface MeshResult {
  readonly positions: Float32Array;
  readonly normals: Float32Array;
  readonly indices: Uint32Array;
  readonly colors: Float32Array;
  readonly triangleCount: number;
  readonly vertexCount: number;
}

export interface VoxelMeshWasm {
  mesh_from_positions(
    positions: Float32Array,
    voxelSize: number,
    materialId: number
  ): MeshResult;

  mesh_chunk(
    chunkData: Uint8Array,
    neighborData: Uint8Array,
    voxelSize: number
  ): MeshResult;
}
```

---

## 6. Coordinate System Reference

### Diagram

```
                    WORLD SPACE (f32)
                    ════════════════
                         +Y (up)
                          │
                          │
                          │
                          │
          ────────────────┼────────────────→ +X
                         /│
                        / │
                       /  │
                      /   │
                     +Z (forward)

    World position: [x, y, z] in f32
    Units: arbitrary (meters, etc.)


                    VOXEL INDEX (i32)
                    ═════════════════
    voxel_index = floor(world_pos / voxel_size)

    Examples (voxel_size = 0.1):
      world [0.05, 0.15, 0.25] → voxel [0, 1, 2]
      world [-0.05, 0.0, 0.0]  → voxel [-1, 0, 0]


                    CHUNK COORD (i32)
                    ═════════════════
    chunk_coord = floor(voxel_index / chunk_size)

    Examples (chunk_size = 64):
      voxel [0, 0, 0]    → chunk [0, 0, 0]
      voxel [63, 63, 63] → chunk [0, 0, 0]
      voxel [64, 0, 0]   → chunk [1, 0, 0]
      voxel [-1, 0, 0]   → chunk [-1, 0, 0]


                    LOCAL COORD (u32)
                    ═════════════════
    local = voxel_index mod chunk_size (always 0..chunk_size-1)

    Examples (chunk_size = 64):
      voxel [0, 0, 0]    → local [0, 0, 0]
      voxel [65, 2, 5]   → local [1, 2, 5]
      voxel [-1, 0, 0]   → local [63, 0, 0]  // Wraps into previous chunk
```

### Conversion Functions

```rust
/// Chunk size for binary greedy meshing (64³ with 1-voxel padding = 62³ usable)
pub const CHUNK_SIZE: u32 = 64;
pub const CHUNK_SIZE_USABLE: u32 = 62;  // After padding

/// World position → Voxel index
pub fn world_to_voxel(pos: [f32; 3], voxel_size: f32) -> [i32; 3] {
    [
        (pos[0] / voxel_size).floor() as i32,
        (pos[1] / voxel_size).floor() as i32,
        (pos[2] / voxel_size).floor() as i32,
    ]
}

/// Voxel index → Chunk coordinate
pub fn voxel_to_chunk(voxel: [i32; 3]) -> ChunkCoord {
    let cs = CHUNK_SIZE as i32;
    ChunkCoord {
        x: voxel[0].div_euclid(cs),
        y: voxel[1].div_euclid(cs),
        z: voxel[2].div_euclid(cs),
    }
}

/// Voxel index → Local coordinate within chunk
pub fn voxel_to_local(voxel: [i32; 3]) -> [u32; 3] {
    let cs = CHUNK_SIZE as i32;
    [
        voxel[0].rem_euclid(cs) as u32,
        voxel[1].rem_euclid(cs) as u32,
        voxel[2].rem_euclid(cs) as u32,
    ]
}

/// Chunk coordinate + local → Voxel index
pub fn chunk_local_to_voxel(chunk: ChunkCoord, local: [u32; 3]) -> [i32; 3] {
    let cs = CHUNK_SIZE as i32;
    [
        chunk.x * cs + local[0] as i32,
        chunk.y * cs + local[1] as i32,
        chunk.z * cs + local[2] as i32,
    ]
}

/// Voxel index → World position (voxel center)
pub fn voxel_to_world_center(voxel: [i32; 3], voxel_size: f32) -> [f32; 3] {
    [
        (voxel[0] as f32 + 0.5) * voxel_size,
        (voxel[1] as f32 + 0.5) * voxel_size,
        (voxel[2] as f32 + 0.5) * voxel_size,
    ]
}

/// Voxel index → World position (voxel min corner)
pub fn voxel_to_world_min(voxel: [i32; 3], voxel_size: f32) -> [f32; 3] {
    [
        voxel[0] as f32 * voxel_size,
        voxel[1] as f32 * voxel_size,
        voxel[2] as f32 * voxel_size,
    ]
}

/// Chunk coordinate → World position (chunk min corner)
pub fn chunk_to_world_min(chunk: ChunkCoord, voxel_size: f32) -> [f32; 3] {
    let cs = CHUNK_SIZE as f32;
    [
        chunk.x as f32 * cs * voxel_size,
        chunk.y as f32 * cs * voxel_size,
        chunk.z as f32 * cs * voxel_size,
    ]
}
```

### Important: Negative Coordinates

Use `div_euclid` and `rem_euclid` for correct handling of negative indices:

```rust
// WRONG: Standard division/modulo
let x = -1i32;
let chunk_size = 64i32;
println!("{}", x / chunk_size);   // 0 (wrong! should be -1)
println!("{}", x % chunk_size);   // -1 (wrong! should be 63)

// CORRECT: Euclidean division/modulo
println!("{}", x.div_euclid(chunk_size));  // -1 ✓
println!("{}", x.rem_euclid(chunk_size));  // 63 ✓
```

---

## 7. WASM-to-GPU Data Format (64-bit Limitation)

### The Problem

WGSL (WebGPU Shading Language) does not support 64-bit integers (`i64`/`u64`). Our binary greedy meshing algorithm uses 64-bit bitmasks extensively in WASM for processing 64 voxels in parallel.

**Impact**: 64-bit packed quad format cannot be sent directly to GPU shaders.

### Solution: Expand Before GPU Transfer

The 64-bit packed quad format is an **internal optimization** for the Rust/WASM meshing pipeline. Before returning data to JavaScript (and ultimately the GPU), we expand to standard 32-bit vertex arrays.

```
┌─────────────────────────────────────────────────────────────────────────┐
│                    DATA FLOW WITH FORMAT CONVERSION                     │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  WASM (Rust)                          │  JavaScript / GPU               │
│  ═══════════                          │  ════════════════               │
│                                       │                                 │
│  ┌───────────────┐                    │                                 │
│  │ Opaque Mask   │  64-bit bitmasks   │                                 │
│  │ [u64; CS_P²]  │  (internal only)   │                                 │
│  └───────┬───────┘                    │                                 │
│          │                            │                                 │
│          ▼                            │                                 │
│  ┌───────────────┐                    │                                 │
│  │ Face Culling  │  Bitwise ops on    │                                 │
│  │ (64 at once)  │  64-bit masks      │                                 │
│  └───────┬───────┘                    │                                 │
│          │                            │                                 │
│          ▼                            │                                 │
│  ┌───────────────┐                    │                                 │
│  │ Packed Quads  │  64-bit packed     │                                 │
│  │ Vec<u64>      │  (position+size+   │                                 │
│  │               │   material)        │                                 │
│  └───────┬───────┘                    │                                 │
│          │                            │                                 │
│          ▼                            │                                 │
│  ┌───────────────┐                    │  ┌───────────────┐              │
│  │ Quad Expander │  Unpack to         │  │ Float32Array  │  Standard    │
│  │ (in WASM)     │  vertices ────────────▶ positions     │  32-bit      │
│  └───────────────┘                    │  │ Float32Array  │  arrays      │
│                                       │  │ normals       │              │
│                                       │  │ Uint32Array   │              │
│                                       │  │ indices       │              │
│                                       │  └───────┬───────┘              │
│                                       │          │                      │
│                                       │          ▼                      │
│                                       │  ┌───────────────┐              │
│                                       │  │ BufferGeometry│  GPU-ready   │
│                                       │  │ (Three.js)    │  buffers     │
│                                       │  └───────────────┘              │
│                                       │                                 │
└─────────────────────────────────────────────────────────────────────────┘
```

### Implementation Details

#### Packed Quad Format (Internal, 64-bit)

```rust
// Used ONLY within WASM for efficient storage during meshing
// Bit layout:
// - Bits 0-5:   X position (6 bits, 0-63)
// - Bits 6-11:  Y position (6 bits, 0-63)
// - Bits 12-17: Z position (6 bits, 0-63)
// - Bits 18-23: Width (6 bits, 1-62)
// - Bits 24-29: Height (6 bits, 1-62)
// - Bits 30-31: Reserved
// - Bits 32-63: Material ID (32 bits)
type PackedQuad = u64;
```

#### Expanded Format (External, 32-bit compatible)

```rust
/// Output format - all 32-bit arrays, GPU-compatible
pub struct MeshOutput {
    pub positions: Vec<f32>,  // 3 floats per vertex
    pub normals: Vec<f32>,    // 3 floats per vertex
    pub indices: Vec<u32>,    // 3 indices per triangle
    pub colors: Vec<f32>,     // 3 floats per vertex (RGB)
}
```

#### Expansion Function

```rust
/// Convert packed quads to standard vertex arrays
fn expand_quad(quad: u64, face: usize, voxel_size: f32, origin: [f32; 3]) -> QuadVertices {
    let (x, y, z, w, h, material) = unpack_quad(quad);

    // Generate 4 corner positions (12 floats)
    let corners = generate_corners(face, x, y, z, w, h, voxel_size, origin);

    // All 4 vertices share the same normal (6 floats repeated 4x)
    let normal = FACE_NORMALS[face];

    // Look up color from material palette (3 floats repeated 4x)
    let color = palette.get(material).color;

    QuadVertices { corners, normal, color }
}
```

### Why Not Two u32 Values?

An alternative would be to pass the 64-bit packed quads as two `u32` values and reconstruct in the shader. This was rejected because:

1. **Shader complexity**: Reconstructing from two u32s adds branching and bit manipulation in the shader
2. **Vertex count unchanged**: We still need 4 vertices per quad regardless of format
3. **Expansion is fast**: The expand step is ~10% of total meshing time
4. **Standard pipeline**: Three.js expects standard vertex arrays, not custom formats

### Performance Characteristics

| Stage | Time (64³ terrain) | Output Size |
|-------|-------------------|-------------|
| Face culling (64-bit ops) | ~30 µs | 192 KB (6 × 32KB masks) |
| Greedy merge | ~35 µs | ~2000 × 8 bytes = 16 KB |
| Quad expansion | ~9 µs | ~40 KB (positions+normals+indices) |
| **Total** | **~74 µs** | **~40 KB** |

The 64-bit optimization provides 10-50x speedup in the critical face culling step while the expansion step adds minimal overhead.

---

## 8. WASM Memory Safety

### The Problem

The `js_sys::Float32Array::view()` function creates a view directly into WASM linear memory:

```rust
// In WASM bindings
#[wasm_bindgen(getter)]
pub fn positions(&self) -> js_sys::Float32Array {
    unsafe { js_sys::Float32Array::view(&self.positions) }
}
```

**Hazard**: If the WASM heap grows (due to allocation) after the view is created, the view's backing memory may be invalidated, causing undefined behavior.

### Solution: Copy-On-Access Pattern

For safety, JavaScript code should immediately copy the data:

```typescript
// SAFE: Copy data immediately after access
const meshResult = wasm.mesh_voxel_positions(positions, voxelSize, materialId);
const positionsCopy = new Float32Array(meshResult.positions);  // Copy immediately
const normalsCopy = new Float32Array(meshResult.normals);
const indicesCopy = new Uint32Array(meshResult.indices);

// Now safe to do any operation that might trigger WASM allocation
const anotherResult = wasm.mesh_voxel_positions(...);  // This might grow heap

// Use the copies for Three.js
geometry.setAttribute('position', new BufferAttribute(positionsCopy, 3));
```

```typescript
// UNSAFE: Storing view without copying
const meshResult = wasm.mesh_voxel_positions(...);
const positionsView = meshResult.positions;  // Just a view, not a copy

const anotherResult = wasm.mesh_voxel_positions(...);  // Heap grows!

// positionsView may now point to invalid memory
geometry.setAttribute('position', new BufferAttribute(positionsView, 3));  // UB!
```

### TypeScript Wrapper

```typescript
/**
 * Wrapper that ensures safe copies of WASM array views.
 * Call this immediately after receiving mesh results from WASM.
 */
export function copyMeshResult(result: WasmMeshResult): MeshData {
    return {
        positions: new Float32Array(result.positions),
        normals: new Float32Array(result.normals),
        indices: new Uint32Array(result.indices),
        colors: result.colors ? new Float32Array(result.colors) : undefined,
    };
}

// Usage in VoxelizerAdapter
export class VoxelizerAdapter {
    meshVoxelPositions(positions: Float32Array, voxelSize: number): MeshData {
        const result = this.wasm.mesh_voxel_positions(positions, voxelSize, 0);
        return copyMeshResult(result);  // Safe copy before any other WASM calls
    }
}
```

### Alternative: Clone in Rust

For maximum safety, clone the data in Rust before returning:

```rust
#[wasm_bindgen(getter)]
pub fn positions(&self) -> Vec<f32> {
    self.positions.clone()  // Allocates new memory, always safe
}
```

**Trade-off**: This doubles memory usage and adds copy overhead, but eliminates the safety concern entirely.

### Recommendation

Use the **Copy-On-Access Pattern** in the TypeScript wrapper. This:
- Maintains performance (zero-copy inside WASM)
- Provides safety at the API boundary
- Is explicit about ownership transfer

---

## 9. Cross-Language Logging

### The Problem

Debugging a system that spans Rust/WASM and TypeScript requires unified logging that:
1. Works in both languages
2. Has consistent log levels and formatting
3. Supports performance timing across language boundaries
4. Can be enabled/disabled without recompilation
5. Doesn't impact production performance

### Log Levels

| Level | Purpose | Example |
|-------|---------|---------|
| `ERROR` | Failures requiring attention | "Failed to allocate mesh buffer" |
| `WARN` | Recoverable issues | "Chunk version mismatch, re-queuing" |
| `INFO` | Major state changes | "Rebuild queue processed 4 chunks" |
| `DEBUG` | Detailed operations | "Face culling produced 1234 faces" |
| `TRACE` | Fine-grained tracing | "Processing column (32, 45)" |
| `PERF` | Performance timing | "greedy_merge: 35.2µs" |

### Rust Logging (WASM)

Use the `log` crate facade with `console_log` backend for WASM:

```rust
// Cargo.toml
[dependencies]
log = "0.4"
console_log = { version = "1.0", features = ["color"] }

// Optional: compile out logs in release
[features]
debug-logs = ["log/max_level_debug"]
trace-logs = ["log/max_level_trace"]
```

```rust
use log::{debug, info, warn, error, trace};

/// Initialize logging - call once from JS
#[wasm_bindgen]
pub fn init_logging(level: &str) {
    let log_level = match level {
        "error" => log::LevelFilter::Error,
        "warn" => log::LevelFilter::Warn,
        "info" => log::LevelFilter::Info,
        "debug" => log::LevelFilter::Debug,
        "trace" => log::LevelFilter::Trace,
        _ => log::LevelFilter::Info,
    };

    console_log::init_with_level(log_level)
        .expect("Failed to initialize logger");

    info!("WASM logging initialized at level: {}", level);
}

/// Example usage in meshing code
pub fn greedy_mesh(chunk: &BinaryChunk) -> Vec<PackedQuad> {
    debug!("Starting greedy mesh for chunk");

    let faces = cull_faces(chunk);
    debug!("Face culling complete: {} total faces",
           faces.iter().map(|f| f.count_ones()).sum::<u32>());

    let quads = merge_faces(&faces);
    info!("Greedy merge: {} faces → {} quads",
          faces.iter().map(|f| f.count_ones()).sum::<u32>(),
          quads.len());

    quads
}
```

### TypeScript Logging

Create a unified logger that matches the Rust format:

```typescript
// logger.ts
export type LogLevel = 'error' | 'warn' | 'info' | 'debug' | 'trace' | 'perf';

interface LogConfig {
  level: LogLevel;
  enablePerf: boolean;
  prefix: string;
}

const LEVEL_PRIORITY: Record<LogLevel, number> = {
  error: 0,
  warn: 1,
  info: 2,
  debug: 3,
  trace: 4,
  perf: 5,
};

class Logger {
  private config: LogConfig = {
    level: 'info',
    enablePerf: false,
    prefix: '[Gestalt]',
  };

  configure(config: Partial<LogConfig>): void {
    this.config = { ...this.config, ...config };
  }

  private shouldLog(level: LogLevel): boolean {
    if (level === 'perf') return this.config.enablePerf;
    return LEVEL_PRIORITY[level] <= LEVEL_PRIORITY[this.config.level];
  }

  private format(level: LogLevel, module: string, message: string): string {
    const timestamp = new Date().toISOString().split('T')[1].slice(0, -1);
    return `${timestamp} ${this.config.prefix} [${level.toUpperCase()}] ${module}: ${message}`;
  }

  error(module: string, message: string, ...args: unknown[]): void {
    if (this.shouldLog('error')) {
      console.error(this.format('error', module, message), ...args);
    }
  }

  warn(module: string, message: string, ...args: unknown[]): void {
    if (this.shouldLog('warn')) {
      console.warn(this.format('warn', module, message), ...args);
    }
  }

  info(module: string, message: string, ...args: unknown[]): void {
    if (this.shouldLog('info')) {
      console.info(this.format('info', module, message), ...args);
    }
  }

  debug(module: string, message: string, ...args: unknown[]): void {
    if (this.shouldLog('debug')) {
      console.debug(this.format('debug', module, message), ...args);
    }
  }

  trace(module: string, message: string, ...args: unknown[]): void {
    if (this.shouldLog('trace')) {
      console.log(this.format('trace', module, message), ...args);
    }
  }

  // Performance timing with auto-logging
  perf<T>(module: string, operation: string, fn: () => T): T {
    if (!this.config.enablePerf) {
      return fn();
    }

    const start = performance.now();
    const result = fn();
    const elapsed = performance.now() - start;

    console.log(
      this.format('perf', module, `${operation}: ${elapsed.toFixed(2)}ms`)
    );

    return result;
  }

  // Async version
  async perfAsync<T>(
    module: string,
    operation: string,
    fn: () => Promise<T>
  ): Promise<T> {
    if (!this.config.enablePerf) {
      return fn();
    }

    const start = performance.now();
    const result = await fn();
    const elapsed = performance.now() - start;

    console.log(
      this.format('perf', module, `${operation}: ${elapsed.toFixed(2)}ms`)
    );

    return result;
  }
}

export const logger = new Logger();
```

### Usage Examples

```typescript
// ChunkManager.ts
import { logger } from './logger';

export class ChunkManager {
  private readonly MODULE = 'ChunkManager';

  setVoxel(x: number, y: number, z: number, value: number): void {
    logger.debug(this.MODULE, `setVoxel(${x}, ${y}, ${z}, ${value})`);

    const coord = this.worldToChunk(x, y, z);
    const chunk = this.getOrCreateChunk(coord);

    chunk.setVoxel(x % CHUNK_SIZE, y % CHUNK_SIZE, z % CHUNK_SIZE, value);

    // Check if boundary edit
    if (this.isBoundaryVoxel(x, y, z)) {
      logger.debug(this.MODULE, `Boundary edit at (${x}, ${y}, ${z}), marking neighbors dirty`);
      this.markNeighborsDirty(coord);
    }

    this.markDirty(coord);
  }

  processRebuilds(): void {
    logger.perf(this.MODULE, 'processRebuilds', () => {
      const batch = this.rebuildQueue.takeBatch(this.config.maxPerFrame);
      logger.info(this.MODULE, `Processing ${batch.length} chunks`);

      for (const coord of batch) {
        this.rebuildChunk(coord);
      }
    });
  }

  private rebuildChunk(coord: ChunkCoord): void {
    const chunk = this.chunks.get(coordKey(coord));
    if (!chunk) {
      logger.warn(this.MODULE, `Chunk ${coordKey(coord)} not found for rebuild`);
      return;
    }

    logger.perf(this.MODULE, `mesh chunk ${coordKey(coord)}`, () => {
      const meshData = this.wasm.meshBinaryChunk(chunk.data);
      this.applyMesh(coord, meshData);
    });
  }
}
```

### Cross-Language Performance Tracing

For timing operations that span WASM and JS:

```typescript
// perf-tracer.ts
interface PerfSpan {
  name: string;
  start: number;
  end?: number;
  children: PerfSpan[];
  metadata?: Record<string, unknown>;
}

class PerfTracer {
  private enabled = false;
  private rootSpan: PerfSpan | null = null;
  private spanStack: PerfSpan[] = [];

  enable(): void {
    this.enabled = true;
  }

  disable(): void {
    this.enabled = false;
  }

  startSpan(name: string, metadata?: Record<string, unknown>): void {
    if (!this.enabled) return;

    const span: PerfSpan = {
      name,
      start: performance.now(),
      children: [],
      metadata,
    };

    if (this.spanStack.length === 0) {
      this.rootSpan = span;
    } else {
      this.spanStack[this.spanStack.length - 1].children.push(span);
    }

    this.spanStack.push(span);
  }

  endSpan(): void {
    if (!this.enabled || this.spanStack.length === 0) return;

    const span = this.spanStack.pop()!;
    span.end = performance.now();
  }

  // Use with WASM calls
  async traceWasm<T>(
    name: string,
    wasmFn: () => T,
    metadata?: Record<string, unknown>
  ): Promise<T> {
    this.startSpan(name, { ...metadata, source: 'wasm' });
    try {
      return wasmFn();
    } finally {
      this.endSpan();
    }
  }

  getTrace(): PerfSpan | null {
    return this.rootSpan;
  }

  printTrace(): void {
    if (!this.rootSpan) return;

    const print = (span: PerfSpan, indent: number): void => {
      const duration = span.end ? (span.end - span.start).toFixed(2) : '?';
      const prefix = '  '.repeat(indent);
      console.log(`${prefix}${span.name}: ${duration}ms`);

      for (const child of span.children) {
        print(child, indent + 1);
      }
    };

    print(this.rootSpan, 0);
  }

  reset(): void {
    this.rootSpan = null;
    this.spanStack = [];
  }
}

export const perfTracer = new PerfTracer();
```

### Usage: End-to-End Tracing

```typescript
// Example: Trace full edit → rebuild → render cycle
async function editWithTrace(x: number, y: number, z: number, value: number) {
  perfTracer.enable();
  perfTracer.startSpan('edit-cycle', { x, y, z, value });

  try {
    // JS: Mark dirty
    perfTracer.startSpan('mark-dirty');
    chunkManager.setVoxel(x, y, z, value);
    perfTracer.endSpan();

    // WASM: Mesh
    perfTracer.startSpan('rebuild-all');
    for (const coord of chunkManager.getDirtyChunks()) {
      await perfTracer.traceWasm(
        `mesh-${coordKey(coord)}`,
        () => wasm.meshBinaryChunk(chunkManager.getChunk(coord).data),
        { coord }
      );
    }
    perfTracer.endSpan();

    // JS: Apply to GPU
    perfTracer.startSpan('gpu-upload');
    renderer.render(scene, camera);
    perfTracer.endSpan();

  } finally {
    perfTracer.endSpan(); // edit-cycle
    perfTracer.printTrace();
    perfTracer.reset();
  }
}

// Output:
// edit-cycle: 12.45ms
//   mark-dirty: 0.12ms
//   rebuild-all: 8.23ms
//     mesh-(0,0,0): 3.21ms
//     mesh-(1,0,0): 2.89ms
//     mesh-(0,1,0): 2.13ms
//   gpu-upload: 4.10ms
```

### Configuration

```typescript
// Initialize both loggers on startup
import { logger } from './logger';
import { initWasmLogging } from './wasm-adapter';

export function initializeLogging(config: {
  level: LogLevel;
  enablePerf: boolean;
}): void {
  // Configure TypeScript logger
  logger.configure({
    level: config.level,
    enablePerf: config.enablePerf,
    prefix: '[Gestalt]',
  });

  // Configure WASM logger (must match TypeScript level)
  initWasmLogging(config.level);

  logger.info('App', `Logging initialized at level: ${config.level}`);
}

// Development defaults
if (import.meta.env.DEV) {
  initializeLogging({ level: 'debug', enablePerf: true });
} else {
  initializeLogging({ level: 'warn', enablePerf: false });
}
```

### Log Output Format

Both languages produce consistent output:

```
14:23:45.123 [Gestalt] [INFO] ChunkManager: Processing 4 chunks
14:23:45.125 [Gestalt] [DEBUG] ChunkManager: mesh chunk (0,0,0)
14:23:45.128 [Gestalt] [PERF] ChunkManager: mesh chunk (0,0,0): 3.21ms
// From WASM (via console_log):
14:23:45.126 [INFO] greedy_mesh: Greedy merge: 1234 faces → 456 quads
14:23:45.127 [DEBUG] expand_quads: Expanded 456 quads to 1824 vertices
```

### Conditional Compilation

For release builds, strip debug/trace logs:

```rust
// Cargo.toml
[features]
default = []
debug-logs = []

[profile.release]
# Rust will DCE (dead code eliminate) unused log calls
```

```typescript
// vite.config.ts
export default defineConfig({
  define: {
    __DEV__: JSON.stringify(process.env.NODE_ENV !== 'production'),
  },
  esbuild: {
    drop: process.env.NODE_ENV === 'production' ? ['console', 'debugger'] : [],
  },
});
```

---

## Summary

| Gap | Solution |
|-----|----------|
| Cross-chunk boundaries | PaddedChunkView with 1-voxel neighbor slices |
| Voxelizer → Chunks | Converter: positions → chunked grid |
| Material strategy | Vertex colors (start), palette texture (later) |
| Migration path | Feature flag, parallel implementation, phased rollout |
| WASM API | Unified API for both full mesh and per-chunk |
| Coordinate systems | Clear diagram + conversion functions |
| WebGPU 64-bit limitation | Expand packed quads to 32-bit arrays in WASM before JS transfer |
| WASM memory safety | Copy-on-access pattern with TypeScript wrapper |
| Cross-language logging | Unified logger with log/console_log crates + TS wrapper |
