# Binary Greedy Mesh Implementation

> **Part of the Voxel Mesh Architecture**
>
> This document details the binary greedy meshing algorithm using bitwise operations for 10-50x speedup.
>
> Related documents:
> - [Architecture Overview](voxel-mesh-architecture.md) - High-level system design
> - [Binary Greedy Meshing Analysis](binary-greedy-meshing-analysis.md) - Algorithm deep-dive and reference implementation
> - [Chunk Management System](chunk-management-system.md) - Dirty tracking, rebuild queue, state machine
> - [Three.js Buffer Management](threejs-buffer-management.md) - GPU buffer lifecycle, double-buffering

---

## Overview

Binary greedy meshing uses bitwise operations to process 64 voxels in parallel. Instead of iterating voxel-by-voxel, we encode voxel columns as 64-bit integers and perform visibility culling and merging using bit manipulation.

**Performance comparison:**

| Approach | Time per 64³ chunk | Speedup |
|----------|-------------------|---------|
| Traditional greedy mesh | 1-5 ms | 1x |
| Binary greedy mesh | 50-200 µs | 10-50x |

**Triangle count comparison for a 100³ solid cube:**

| Method | Triangles |
|--------|-----------|
| Instanced cubes (all voxels) | 6,000,000 |
| Face culling (visible faces only) | ~60,000 |
| Greedy meshing | 12 |

---

## Part 1: Data Structures

### Chunk Constants

```rust
/// Chunk size with 1-voxel padding on each side
/// Internal: 64³, Usable: 62³ (padding allows neighbor lookups without bounds checks)
pub const CS_P: usize = 64;  // Chunk Size with Padding
pub const CS: usize = 62;    // Chunk Size (usable)
pub const CS_P2: usize = CS_P * CS_P;  // Slice size
pub const CS_P3: usize = CS_P * CS_P * CS_P;  // Total voxels
```

### Binary Chunk Representation

```rust
/// 16-bit material identifier (see ADR-0007 for material strategy)
pub type MaterialId = u16;

/// Reserved material values
pub const MATERIAL_EMPTY: MaterialId = 0;
pub const MATERIAL_DEFAULT: MaterialId = 1;

/// Binary representation of a chunk for fast meshing
pub struct BinaryChunk {
    /// Opaque mask: one bit per voxel, organized as vertical columns
    /// opaque_mask[x * CS_P + z] contains 64 bits for the Y column at (x, z)
    pub opaque_mask: [u64; CS_P2],

    /// Material IDs: 16-bit per voxel (only read for visible faces)
    /// Supports 65536 materials for texture atlas indexing
    pub materials: [MaterialId; CS_P3],
}

impl BinaryChunk {
    pub fn new() -> Self {
        Self {
            opaque_mask: [0u64; CS_P2],
            materials: [MATERIAL_EMPTY; CS_P3],
        }
    }

    /// Set a voxel as solid with given material
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, z: usize, material: MaterialId) {
        let column_idx = x * CS_P + z;
        self.opaque_mask[column_idx] |= 1u64 << y;
        self.materials[x * CS_P2 + y * CS_P + z] = material;
    }

    /// Check if voxel is solid
    #[inline]
    pub fn is_solid(&self, x: usize, y: usize, z: usize) -> bool {
        let column_idx = x * CS_P + z;
        (self.opaque_mask[column_idx] >> y) & 1 != 0
    }

    /// Get material at position (only valid if solid)
    #[inline]
    pub fn get_material(&self, x: usize, y: usize, z: usize) -> MaterialId {
        self.materials[x * CS_P2 + y * CS_P + z]
    }
}
```

### Face Masks Storage

```rust
/// Face masks for all 6 directions
/// Each mask indicates which faces are visible after culling
pub struct FaceMasks {
    /// 6 directions × CS_P² columns
    /// facemasks[face * CS_P2 + column_idx] = 64-bit mask of visible faces
    pub masks: [u64; 6 * CS_P2],
}

impl FaceMasks {
    pub fn new() -> Self {
        Self { masks: [0u64; 6 * CS_P2] }
    }

    #[inline]
    pub fn get(&self, face: usize, x: usize, z: usize) -> u64 {
        self.masks[face * CS_P2 + x * CS_P + z]
    }

    #[inline]
    pub fn set(&mut self, face: usize, x: usize, z: usize, mask: u64) {
        self.masks[face * CS_P2 + x * CS_P + z] = mask;
    }
}
```

### Quad Output (Packed Format)

```rust
/// Packed quad format: 64 bits total
///
/// Bit layout:
/// - Bits 0-5:   X position (6 bits, 0-63)
/// - Bits 6-11:  Y position (6 bits, 0-63)
/// - Bits 12-17: Z position (6 bits, 0-63)
/// - Bits 18-23: Width (6 bits, 1-62)
/// - Bits 24-29: Height (6 bits, 1-62)
/// - Bits 30-31: Unused
/// - Bits 32-63: Material ID (32 bits)
#[inline]
pub fn pack_quad(x: u32, y: u32, z: u32, w: u32, h: u32, material: u32) -> u64 {
    ((material as u64) << 32)
        | ((h as u64) << 24)
        | ((w as u64) << 18)
        | ((z as u64) << 12)
        | ((y as u64) << 6)
        | (x as u64)
}

/// Unpack quad components
#[inline]
pub fn unpack_quad(quad: u64) -> (u32, u32, u32, u32, u32, u32) {
    let x = (quad & 0x3F) as u32;
    let y = ((quad >> 6) & 0x3F) as u32;
    let z = ((quad >> 12) & 0x3F) as u32;
    let w = ((quad >> 18) & 0x3F) as u32;
    let h = ((quad >> 24) & 0x3F) as u32;
    let material = (quad >> 32) as u32;
    (x, y, z, w, h, material)
}
```

### Mesh Output

```rust
/// Output mesh data ready for Three.js BufferGeometry
/// Includes UV coordinates and material IDs for texture atlas rendering
/// See ADR-0007 for material strategy details
#[derive(Default)]
pub struct MeshOutput {
    /// Vertex positions (3 floats per vertex)
    pub positions: Vec<f32>,
    /// Vertex normals (3 floats per vertex)
    pub normals: Vec<f32>,
    /// Triangle indices (3 indices per triangle)
    pub indices: Vec<u32>,
    /// UV coordinates (2 floats per vertex)
    /// Tiled appropriately for merged quads (4x3 quad = 4x3 UV tiles)
    pub uvs: Vec<f32>,
    /// Per-vertex material ID for shader lookup
    /// All 4 vertices of a quad share the same material
    pub material_ids: Vec<u16>,
}

impl MeshOutput {
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Pre-allocate capacity for estimated quad count
    pub fn with_capacity(estimated_quads: usize) -> Self {
        let verts = estimated_quads * 4;
        let tris = estimated_quads * 2;
        Self {
            positions: Vec::with_capacity(verts * 3),
            normals: Vec::with_capacity(verts * 3),
            indices: Vec::with_capacity(tris * 3),
            uvs: Vec::with_capacity(verts * 2),
            material_ids: Vec::with_capacity(verts),
        }
    }
}
```

---

## Part 2: Input Conversion

Convert from various input formats to binary chunk representation.

### From Position Array

```rust
/// Convert voxel positions (Float32Array from JS) to binary chunk
/// Uses robust_floor for accurate float-to-voxel conversion (see ADR-0008)
pub fn positions_to_binary_chunk(
    positions: &[f32],
    voxel_size: f32,
    chunk_origin: [f32; 3],
    material_id: MaterialId,
) -> BinaryChunk {
    let mut chunk = BinaryChunk::new();
    let inv_voxel_size = 1.0 / voxel_size;

    for pos in positions.chunks_exact(3) {
        // Convert world position to chunk-local voxel coordinates
        // Use robust_floor to handle floating point edge cases
        let x = robust_floor((pos[0] - chunk_origin[0]) * inv_voxel_size) as usize + 1;
        let y = robust_floor((pos[1] - chunk_origin[1]) * inv_voxel_size) as usize + 1;
        let z = robust_floor((pos[2] - chunk_origin[2]) * inv_voxel_size) as usize + 1;

        if x < CS_P - 1 && y < CS_P - 1 && z < CS_P - 1 {
            chunk.set(x, y, z, material_id);
        }
    }

    chunk
}

/// Robust floor that handles values very close to integers
const COORD_EPSILON: f32 = 1e-5;

fn robust_floor(value: f32) -> i32 {
    let rounded = value.round();
    if (value - rounded).abs() < COORD_EPSILON {
        rounded as i32
    } else {
        value.floor() as i32
    }
}
```

### From Dense Voxel Array

```rust
/// Convert dense voxel array to binary chunk
/// Supports 16-bit material IDs for full texture atlas range
pub fn dense_to_binary_chunk(
    voxels: &[u16],  // 16-bit material_id per voxel, 0 = empty
    dims: [usize; 3],
) -> BinaryChunk {
    let mut chunk = BinaryChunk::new();

    for z in 0..dims[2].min(CS) {
        for y in 0..dims[1].min(CS) {
            for x in 0..dims[0].min(CS) {
                let src_idx = z * dims[1] * dims[0] + y * dims[0] + x;
                let material = voxels[src_idx];

                if material != MATERIAL_EMPTY {
                    // +1 offset for padding
                    chunk.set(x + 1, y + 1, z + 1, material);
                }
            }
        }
    }

    chunk
}
```

---

## Part 3: Face Culling (Bitwise)

Face culling identifies which voxel faces are exposed to air. Using bitwise operations, we process 64 faces per operation.

### Face Directions

```rust
/// Face direction indices
pub const FACE_POS_Y: usize = 0;  // +Y (top)
pub const FACE_NEG_Y: usize = 1;  // -Y (bottom)
pub const FACE_POS_X: usize = 2;  // +X (right)
pub const FACE_NEG_X: usize = 3;  // -X (left)
pub const FACE_POS_Z: usize = 4;  // +Z (front)
pub const FACE_NEG_Z: usize = 5;  // -Z (back)

/// Normal vectors for each face
pub const FACE_NORMALS: [[f32; 3]; 6] = [
    [0.0, 1.0, 0.0],   // +Y
    [0.0, -1.0, 0.0],  // -Y
    [1.0, 0.0, 0.0],   // +X
    [-1.0, 0.0, 0.0],  // -X
    [0.0, 0.0, 1.0],   // +Z
    [0.0, 0.0, -1.0],  // -Z
];
```

### Bitwise Culling Algorithm

```rust
/// Generate face masks for all 6 directions using bitwise operations
pub fn cull_faces(chunk: &BinaryChunk, masks: &mut FaceMasks) {
    for x in 1..CS_P - 1 {
        let x_cs_p = x * CS_P;

        for z in 1..CS_P - 1 {
            let column_idx = x_cs_p + z;
            let column = chunk.opaque_mask[column_idx];

            // Skip empty columns
            if column == 0 {
                continue;
            }

            // +Y face: visible where this voxel is solid AND voxel above is empty
            // Shift column right to compare with y+1
            let pos_y = column & !(column >> 1);
            masks.set(FACE_POS_Y, x, z, (pos_y >> 1) & ((1u64 << CS) - 1));

            // -Y face: visible where this voxel is solid AND voxel below is empty
            // Shift column left to compare with y-1
            let neg_y = column & !(column << 1);
            masks.set(FACE_NEG_Y, x, z, (neg_y >> 1) & ((1u64 << CS) - 1));

            // +X face: compare with x+1 column
            let neighbor_pos_x = chunk.opaque_mask[(x + 1) * CS_P + z];
            let pos_x = column & !neighbor_pos_x;
            masks.set(FACE_POS_X, x, z, (pos_x >> 1) & ((1u64 << CS) - 1));

            // -X face: compare with x-1 column
            let neighbor_neg_x = chunk.opaque_mask[(x - 1) * CS_P + z];
            let neg_x = column & !neighbor_neg_x;
            masks.set(FACE_NEG_X, x, z, (neg_x >> 1) & ((1u64 << CS) - 1));

            // +Z face: compare with z+1 column
            let neighbor_pos_z = chunk.opaque_mask[x_cs_p + z + 1];
            let pos_z = column & !neighbor_pos_z;
            masks.set(FACE_POS_Z, x, z, (pos_z >> 1) & ((1u64 << CS) - 1));

            // -Z face: compare with z-1 column
            let neighbor_neg_z = chunk.opaque_mask[x_cs_p + z - 1];
            let neg_z = column & !neighbor_neg_z;
            masks.set(FACE_NEG_Z, x, z, (neg_z >> 1) & ((1u64 << CS) - 1));
        }
    }
}
```

**Key insight:** The bit-shift operations (`>> 1`, `<< 1`) effectively sample neighbor voxels for all 64 Y positions simultaneously.

---

## Part 4: Greedy Merging (Bit-Scanning)

After culling, we merge adjacent same-material faces into larger rectangles using bit-scanning intrinsics.

### Bit-Scanning Intrinsic

```rust
/// Find position of lowest set bit (0-63), or 64 if no bits set
#[inline]
fn find_first_set(mask: u64) -> usize {
    if mask == 0 {
        64
    } else {
        mask.trailing_zeros() as usize
    }
}
```

### Greedy Merge for Y-Axis Faces (Top/Bottom)

```rust
/// Greedy merge for +Y/-Y faces
/// These faces span the X-Z plane at each Y slice
pub fn greedy_merge_y_faces(
    face: usize,
    chunk: &BinaryChunk,
    masks: &FaceMasks,
    quads: &mut Vec<u64>,
) {
    // For each Y layer
    for y in 0..CS {
        // Track forward merge state for each X position
        let mut forward_merged: [u8; CS] = [0; CS];

        // Scan Z rows
        for z in 0..CS {
            let mut x = 0;

            while x < CS {
                // Get the face mask for this column
                let col_x = x + 1;  // +1 for padding
                let col_z = z + 1;
                let mask = masks.get(face, col_x, col_z);

                // Check if face exists at this Y
                if (mask >> y) & 1 == 0 {
                    forward_merged[x] = 0;
                    x += 1;
                    continue;
                }

                // Get material at this position
                let material = chunk.get_material(col_x, y + 1, col_z) as u32;

                // === Merge Right (along X) ===
                let mut width = 1usize;
                while x + width < CS {
                    let next_col_x = x + width + 1;
                    let next_mask = masks.get(face, next_col_x, col_z);

                    // Check face exists
                    if (next_mask >> y) & 1 == 0 {
                        break;
                    }

                    // Check same material
                    let next_material = chunk.get_material(next_col_x, y + 1, col_z);
                    if next_material as u32 != material {
                        break;
                    }

                    // Check compatible forward merge
                    if forward_merged[x + width] != forward_merged[x] {
                        break;
                    }

                    width += 1;
                }

                // === Track Forward Merge (along Z) ===
                let height = forward_merged[x] as usize + 1;

                // Update forward merge for all merged positions
                for dx in 0..width {
                    forward_merged[x + dx] = height as u8;
                }

                // === Check if we can merge more in next Z ===
                let can_merge_forward = z + 1 < CS && {
                    let mut can = true;
                    for dx in 0..width {
                        let check_x = x + dx + 1;
                        let check_mask = masks.get(face, check_x, col_z + 1);
                        if (check_mask >> y) & 1 == 0 {
                            can = false;
                            break;
                        }
                        let check_mat = chunk.get_material(check_x, y + 1, col_z + 1);
                        if check_mat as u32 != material {
                            can = false;
                            break;
                        }
                    }
                    can
                };

                // === Emit Quad if merge complete ===
                if !can_merge_forward {
                    let quad = pack_quad(
                        x as u32,
                        y as u32,
                        (z + 1 - height) as u32,
                        width as u32,
                        height as u32,
                        material,
                    );
                    quads.push(quad);

                    // Reset forward merge
                    for dx in 0..width {
                        forward_merged[x + dx] = 0;
                    }
                }

                x += width;
            }
        }
    }
}
```

### Greedy Merge for X-Axis Faces (Left/Right)

```rust
/// Greedy merge for +X/-X faces
/// These faces span the Y-Z plane at each X slice
pub fn greedy_merge_x_faces(
    face: usize,
    chunk: &BinaryChunk,
    masks: &FaceMasks,
    quads: &mut Vec<u64>,
) {
    // For each X layer
    for x in 0..CS {
        let col_x = x + 1;

        // Track forward merge state for each Y position
        let mut forward_merged: [u8; CS] = [0; CS];

        // Scan Z rows
        for z in 0..CS {
            let col_z = z + 1;
            let mut bits = masks.get(face, col_x, col_z);

            while bits != 0 {
                // Find next set bit (visible face)
                let y = find_first_set(bits);
                if y >= CS {
                    break;
                }

                let material = chunk.get_material(col_x, y + 1, col_z) as u32;

                // === Merge Up (along Y) using bit operations ===
                let mut height = 1usize;
                let mut scan_bits = bits >> (y + 1);

                while scan_bits & 1 != 0 && y + height < CS {
                    let next_y = y + height;
                    let next_material = chunk.get_material(col_x, next_y + 1, col_z);

                    if next_material as u32 != material {
                        break;
                    }
                    if forward_merged[next_y] != forward_merged[y] {
                        break;
                    }

                    height += 1;
                    scan_bits >>= 1;
                }

                // === Track Forward Merge (along Z) ===
                let width = forward_merged[y] as usize + 1;
                for dy in 0..height {
                    forward_merged[y + dy] = width as u8;
                }

                // === Check forward merge possibility ===
                let can_merge_forward = z + 1 < CS && {
                    let next_mask = masks.get(face, col_x, col_z + 1);
                    let mut can = true;
                    for dy in 0..height {
                        if (next_mask >> (y + dy)) & 1 == 0 {
                            can = false;
                            break;
                        }
                        let next_mat = chunk.get_material(col_x, y + dy + 1, col_z + 1);
                        if next_mat as u32 != material {
                            can = false;
                            break;
                        }
                    }
                    can
                };

                // === Emit Quad if merge complete ===
                if !can_merge_forward {
                    let quad = pack_quad(
                        x as u32,
                        y as u32,
                        (z + 1 - width) as u32,
                        width as u32,
                        height as u32,
                        material,
                    );
                    quads.push(quad);

                    for dy in 0..height {
                        forward_merged[y + dy] = 0;
                    }
                }

                // Clear processed bits
                bits &= !((1u64 << height) - 1) << y;
            }
        }
    }
}
```

### Greedy Merge for Z-Axis Faces (Front/Back)

```rust
/// Greedy merge for +Z/-Z faces
/// These faces span the X-Y plane at each Z slice
pub fn greedy_merge_z_faces(
    face: usize,
    chunk: &BinaryChunk,
    masks: &FaceMasks,
    quads: &mut Vec<u64>,
) {
    // For each Z layer
    for z in 0..CS {
        let col_z = z + 1;

        // Track forward merge state for each Y position
        let mut forward_merged: [u8; CS] = [0; CS];

        // Scan X rows
        for x in 0..CS {
            let col_x = x + 1;
            let mut bits = masks.get(face, col_x, col_z);

            while bits != 0 {
                let y = find_first_set(bits);
                if y >= CS {
                    break;
                }

                let material = chunk.get_material(col_x, y + 1, col_z) as u32;

                // === Merge Up (along Y) ===
                let mut height = 1usize;
                let mut scan_bits = bits >> (y + 1);

                while scan_bits & 1 != 0 && y + height < CS {
                    let next_material = chunk.get_material(col_x, y + height + 1, col_z);
                    if next_material as u32 != material {
                        break;
                    }
                    if forward_merged[y + height] != forward_merged[y] {
                        break;
                    }
                    height += 1;
                    scan_bits >>= 1;
                }

                // === Track Forward Merge (along X) ===
                let width = forward_merged[y] as usize + 1;
                for dy in 0..height {
                    forward_merged[y + dy] = width as u8;
                }

                // === Check forward merge possibility ===
                let can_merge_forward = x + 1 < CS && {
                    let next_mask = masks.get(face, col_x + 1, col_z);
                    let mut can = true;
                    for dy in 0..height {
                        if (next_mask >> (y + dy)) & 1 == 0 {
                            can = false;
                            break;
                        }
                        let next_mat = chunk.get_material(col_x + 1, y + dy + 1, col_z);
                        if next_mat as u32 != material {
                            can = false;
                            break;
                        }
                    }
                    can
                };

                // === Emit Quad ===
                if !can_merge_forward {
                    let quad = pack_quad(
                        (x + 1 - width) as u32,
                        y as u32,
                        z as u32,
                        width as u32,
                        height as u32,
                        material,
                    );
                    quads.push(quad);

                    for dy in 0..height {
                        forward_merged[y + dy] = 0;
                    }
                }

                bits &= !((1u64 << height) - 1) << y;
            }
        }
    }
}
```

---

## Part 5: Complete Meshing Pipeline

### Main Entry Point

```rust
/// Mesh a binary chunk into geometry
pub fn mesh_chunk(chunk: &BinaryChunk, voxel_size: f32, origin: [f32; 3]) -> MeshOutput {
    // Step 1: Cull faces (bitwise)
    let mut masks = FaceMasks::new();
    cull_faces(chunk, &mut masks);

    // Step 2: Greedy merge each face direction
    let mut packed_quads: [Vec<u64>; 6] = Default::default();

    greedy_merge_y_faces(FACE_POS_Y, chunk, &masks, &mut packed_quads[FACE_POS_Y]);
    greedy_merge_y_faces(FACE_NEG_Y, chunk, &masks, &mut packed_quads[FACE_NEG_Y]);
    greedy_merge_x_faces(FACE_POS_X, chunk, &masks, &mut packed_quads[FACE_POS_X]);
    greedy_merge_x_faces(FACE_NEG_X, chunk, &masks, &mut packed_quads[FACE_NEG_X]);
    greedy_merge_z_faces(FACE_POS_Z, chunk, &masks, &mut packed_quads[FACE_POS_Z]);
    greedy_merge_z_faces(FACE_NEG_Z, chunk, &masks, &mut packed_quads[FACE_NEG_Z]);

    // Step 3: Expand packed quads to standard vertex arrays
    expand_quads_to_mesh(&packed_quads, voxel_size, origin)
}
```

### Quad Expansion (for Three.js Compatibility)

```rust
/// Expand packed quads into standard vertex/index arrays with UVs
fn expand_quads_to_mesh(
    packed_quads: &[Vec<u64>; 6],
    voxel_size: f32,
    origin: [f32; 3],
) -> MeshOutput {
    // Estimate total quads for pre-allocation
    let total_quads: usize = packed_quads.iter().map(|q| q.len()).sum();
    let mut output = MeshOutput::with_capacity(total_quads);

    for (face, quads) in packed_quads.iter().enumerate() {
        let normal = FACE_NORMALS[face];

        for &quad in quads {
            let (x, y, z, w, h, material) = unpack_quad(quad);

            emit_expanded_quad(
                face,
                x, y, z,
                w, h,
                material as MaterialId,
                &normal,
                voxel_size,
                origin,
                &mut output,
            );
        }
    }

    output
}

/// Emit a single quad as 4 vertices with UVs and 6 indices
/// UVs tile based on quad dimensions (a 4x3 quad tiles texture 4x3 times)
fn emit_expanded_quad(
    face: usize,
    x: u32, y: u32, z: u32,
    width: u32, height: u32,
    material: MaterialId,
    normal: &[f32; 3],
    voxel_size: f32,
    origin: [f32; 3],
    output: &mut MeshOutput,
) {
    let base_vertex = output.vertex_count() as u32;

    // World-space base position
    let bx = origin[0] + x as f32 * voxel_size;
    let by = origin[1] + y as f32 * voxel_size;
    let bz = origin[2] + z as f32 * voxel_size;

    // Width and height in world units
    let w = width as f32 * voxel_size;
    let h = height as f32 * voxel_size;

    // UV tiling: quad dimensions determine how many times texture repeats
    // Shader uses fract(uv) to handle tiling and material_id for atlas lookup
    let u_tiles = width as f32;
    let v_tiles = height as f32;

    // Generate 4 corners based on face direction
    // Each face has consistent UV mapping: (0,0) -> (u_tiles, v_tiles)
    let (corners, uvs): ([[f32; 3]; 4], [[f32; 2]; 4]) = match face {
        FACE_POS_Y => (
            [
                [bx, by + voxel_size, bz],
                [bx + w, by + voxel_size, bz],
                [bx + w, by + voxel_size, bz + h],
                [bx, by + voxel_size, bz + h],
            ],
            [[0.0, 0.0], [u_tiles, 0.0], [u_tiles, v_tiles], [0.0, v_tiles]],
        ),
        FACE_NEG_Y => (
            [
                [bx, by, bz],
                [bx, by, bz + h],
                [bx + w, by, bz + h],
                [bx + w, by, bz],
            ],
            [[0.0, 0.0], [0.0, v_tiles], [u_tiles, v_tiles], [u_tiles, 0.0]],
        ),
        FACE_POS_X => (
            [
                [bx + voxel_size, by, bz],
                [bx + voxel_size, by + h, bz],
                [bx + voxel_size, by + h, bz + w],
                [bx + voxel_size, by, bz + w],
            ],
            [[0.0, 0.0], [0.0, v_tiles], [u_tiles, v_tiles], [u_tiles, 0.0]],
        ),
        FACE_NEG_X => (
            [
                [bx, by, bz],
                [bx, by, bz + w],
                [bx, by + h, bz + w],
                [bx, by + h, bz],
            ],
            [[0.0, 0.0], [u_tiles, 0.0], [u_tiles, v_tiles], [0.0, v_tiles]],
        ),
        FACE_POS_Z => (
            [
                [bx, by, bz + voxel_size],
                [bx + w, by, bz + voxel_size],
                [bx + w, by + h, bz + voxel_size],
                [bx, by + h, bz + voxel_size],
            ],
            [[0.0, 0.0], [u_tiles, 0.0], [u_tiles, v_tiles], [0.0, v_tiles]],
        ),
        FACE_NEG_Z => (
            [
                [bx, by, bz],
                [bx, by + h, bz],
                [bx + w, by + h, bz],
                [bx + w, by, bz],
            ],
            [[0.0, 0.0], [0.0, v_tiles], [u_tiles, v_tiles], [u_tiles, 0.0]],
        ),
        _ => unreachable!(),
    };

    // Add vertices with positions, normals, UVs, and material IDs
    for i in 0..4 {
        output.positions.extend_from_slice(&corners[i]);
        output.normals.extend_from_slice(normal);
        output.uvs.extend_from_slice(&uvs[i]);
        output.material_ids.push(material);
    }

    // Add indices (two triangles, CCW winding)
    output.indices.extend_from_slice(&[
        base_vertex,
        base_vertex + 1,
        base_vertex + 2,
        base_vertex,
        base_vertex + 2,
        base_vertex + 3,
    ]);
}
```

---

## Part 6: WASM Bindings

```rust
use wasm_bindgen::prelude::*;

/// Result of meshing operations, exposed to JavaScript
/// Includes UV coordinates and material IDs for texture atlas rendering
#[wasm_bindgen]
pub struct VoxelMeshResult {
    positions: Vec<f32>,
    normals: Vec<f32>,
    indices: Vec<u32>,
    uvs: Vec<f32>,
    material_ids: Vec<u16>,
}

#[wasm_bindgen]
impl VoxelMeshResult {
    #[wasm_bindgen(getter)]
    pub fn positions(&self) -> Vec<f32> {
        self.positions.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn normals(&self) -> Vec<f32> {
        self.normals.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn indices(&self) -> Vec<u32> {
        self.indices.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn uvs(&self) -> Vec<f32> {
        self.uvs.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn material_ids(&self) -> Vec<u16> {
        self.material_ids.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    #[wasm_bindgen(getter)]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Check if mesh is empty (no geometry generated)
    #[wasm_bindgen(getter)]
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// Mesh voxel positions into optimized geometry with UVs
#[wasm_bindgen]
pub fn mesh_voxel_positions(
    positions: &[f32],      // Voxel center positions (x,y,z triples)
    voxel_size: f32,
    material_id: u16,       // 16-bit material ID
    origin_x: f32,
    origin_y: f32,
    origin_z: f32,
) -> VoxelMeshResult {
    let origin = [origin_x, origin_y, origin_z];

    // Convert to binary chunk
    let chunk = positions_to_binary_chunk(positions, voxel_size, origin, material_id);

    // Mesh the chunk
    let mesh = mesh_chunk(&chunk, voxel_size, origin);

    VoxelMeshResult {
        positions: mesh.positions,
        normals: mesh.normals,
        indices: mesh.indices,
        uvs: mesh.uvs,
        material_ids: mesh.material_ids,
    }
}

/// Mesh dense voxel data with per-voxel materials
#[wasm_bindgen]
pub fn mesh_dense_voxels(
    voxels: &[u16],         // 16-bit material ID per voxel (0 = empty)
    width: u32,
    height: u32,
    depth: u32,
    voxel_size: f32,
    origin_x: f32,
    origin_y: f32,
    origin_z: f32,
) -> VoxelMeshResult {
    let dims = [width as usize, height as usize, depth as usize];
    let origin = [origin_x, origin_y, origin_z];

    // Convert to binary chunk
    let chunk = dense_to_binary_chunk(voxels, dims);

    // Mesh the chunk
    let mesh = mesh_chunk(&chunk, voxel_size, origin);

    VoxelMeshResult {
        positions: mesh.positions,
        normals: mesh.normals,
        indices: mesh.indices,
        uvs: mesh.uvs,
        material_ids: mesh.material_ids,
    }
}
```

---

## Part 7: Three.js Integration

```typescript
import {
  BufferGeometry,
  BufferAttribute,
  Mesh,
  ShaderMaterial,
  DataArrayTexture,
  DataTexture,
  RGBAFormat,
  UnsignedByteType,
  FloatType,
  NearestFilter,
  RepeatWrapping,
} from 'three';

/** Mesh data returned from WASM */
interface VoxelMeshData {
  positions: Float32Array;
  normals: Float32Array;
  indices: Uint32Array;
  uvs: Float32Array;
  materialIds: Uint16Array;
}

/** Build BufferGeometry from WASM mesh result */
function buildVoxelGeometry(data: VoxelMeshData): BufferGeometry {
  const geometry = new BufferGeometry();

  geometry.setAttribute('position', new BufferAttribute(data.positions, 3));
  geometry.setAttribute('normal', new BufferAttribute(data.normals, 3));
  geometry.setAttribute('uv', new BufferAttribute(data.uvs, 2));

  // Material ID as float attribute for shader access
  const materialIdFloat = new Float32Array(data.materialIds.length);
  for (let i = 0; i < data.materialIds.length; i++) {
    materialIdFloat[i] = data.materialIds[i];
  }
  geometry.setAttribute('materialId', new BufferAttribute(materialIdFloat, 1));

  geometry.setIndex(new BufferAttribute(data.indices, 1));
  geometry.computeBoundingBox();
  geometry.computeBoundingSphere();

  return geometry;
}

/**
 * Voxel material shader for texture atlas rendering
 * See ADR-0007 for complete material strategy
 */
function createVoxelMaterial(
  atlas: DataArrayTexture,
  materialData: DataTexture
): ShaderMaterial {
  return new ShaderMaterial({
    uniforms: {
      atlas: { value: atlas },
      materialData: { value: materialData },
      atlasLayerCount: { value: atlas.depth },
      tilesPerLayer: { value: 256 }, // 16x16 tiles
    },
    vertexShader: `
      attribute float materialId;

      varying vec2 vUv;
      varying float vMaterialId;
      varying vec3 vNormal;
      varying vec3 vWorldPosition;

      void main() {
        vUv = uv;
        vMaterialId = materialId;
        vNormal = normalize(normalMatrix * normal);

        vec4 worldPos = modelMatrix * vec4(position, 1.0);
        vWorldPosition = worldPos.xyz;

        gl_Position = projectionMatrix * viewMatrix * worldPos;
      }
    `,
    fragmentShader: `
      precision highp float;
      precision highp sampler2DArray;

      uniform sampler2DArray atlas;
      uniform sampler2D materialData;
      uniform float atlasLayerCount;
      uniform float tilesPerLayer;

      varying vec2 vUv;
      varying float vMaterialId;
      varying vec3 vNormal;
      varying vec3 vWorldPosition;

      void main() {
        // Look up material properties from data texture
        // materialData layout: RGBA = (baseR, baseG, baseB, roughness)
        float matLookup = (vMaterialId + 0.5) / 4096.0;
        vec4 matProps = texture2D(materialData, vec2(matLookup, 0.5));

        // Calculate atlas layer and tile position
        float layer = floor(vMaterialId / tilesPerLayer);
        float tileIndex = mod(vMaterialId, tilesPerLayer);
        float tileX = mod(tileIndex, 16.0);
        float tileY = floor(tileIndex / 16.0);

        // Convert UV to atlas coordinates with tiling
        vec2 tileSize = vec2(1.0 / 16.0);
        vec2 tileOffset = vec2(tileX, tileY) * tileSize;
        vec2 tiledUv = fract(vUv) * tileSize + tileOffset;

        // Sample atlas texture
        vec4 texColor = texture(atlas, vec3(tiledUv, layer));

        // Combine base color with texture
        vec3 baseColor = matProps.rgb;
        vec3 finalColor = texColor.rgb * baseColor;

        // Simple diffuse lighting
        vec3 lightDir = normalize(vec3(0.5, 1.0, 0.3));
        float diffuse = max(dot(vNormal, lightDir), 0.0);
        float ambient = 0.3;

        finalColor *= (ambient + diffuse * 0.7);

        gl_FragColor = vec4(finalColor, texColor.a);
      }
    `,
  });
}

/** Build complete voxel mesh with material */
function buildVoxelMesh(
  data: VoxelMeshData,
  atlas: DataArrayTexture,
  materialData: DataTexture
): Mesh {
  const geometry = buildVoxelGeometry(data);
  const material = createVoxelMaterial(atlas, materialData);
  return new Mesh(geometry, material);
}
```

---

## Part 8: Performance Characteristics

### Computational Complexity

| Operation | Traditional | Binary |
|-----------|-------------|--------|
| Face culling | O(n³) | O(n²) (64 voxels per op) |
| Finding visible faces | O(n³) | O(visible) via CTZ |
| Greedy merge | O(n² × layers) | O(visible × log n) |

Where n = chunk dimension (64).

### Memory Usage

| Data | Size |
|------|------|
| Opaque mask | 64 × 64 × 8 = 32 KB |
| Materials (16-bit) | 64³ × 2 = 512 KB |
| Face masks | 6 × 32 KB = 192 KB |
| Packed quads | ~8 bytes per quad |
| **Total per chunk** | ~736 KB working memory |

### Benchmarks (Expected)

| Chunk Content | Time | Quads |
|---------------|------|-------|
| Empty | ~5 µs | 0 |
| Solid cube | ~30 µs | 6 |
| Terrain surface | ~74 µs | 500-2000 |
| Complex caves | ~150 µs | 5000+ |

---

## Summary

| Step | Description |
|------|-------------|
| 1. Input Conversion | Convert positions/dense array to bitmask representation |
| 2. Face Culling | Bitwise AND with neighbor columns (64 faces per op) |
| 3. Greedy Merge | Bit-scanning to find faces, forward-merge tracking |
| 4. Quad Packing | 8-byte packed format (position + dimensions + material) |
| 5. Expansion | Unpack to standard vertex arrays for Three.js |
| 6. WASM Export | Return typed arrays to JavaScript |
| 7. Three.js | Create BufferGeometry from arrays |

This approach achieves 10-50x speedup over traditional greedy meshing while maintaining compatibility with standard Three.js rendering.

---

## Integration with Chunk System

This binary greedy meshing algorithm operates on a single 64³ chunk at a time. For full system integration including:

- Dirty tracking and boundary neighbor propagation
- Rebuild queue with camera-distance prioritization
- Frame-budgeted processing
- Mesh swap protocol

See [Chunk Management System](chunk-management-system.md).

For Three.js-specific buffer management including:

- Stable Mesh object reuse
- Double-buffered geometry swaps
- Clipping planes for slicing

See [Three.js Buffer Management](threejs-buffer-management.md).
