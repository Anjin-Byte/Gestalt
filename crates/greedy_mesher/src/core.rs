//! Core type definitions for the greedy mesher.

/// Material identifier (16-bit for texture atlas support).
/// Supports up to 65536 materials for texture atlas indexing.
pub type MaterialId = u16;

/// Reserved material value for empty voxels.
pub const MATERIAL_EMPTY: MaterialId = 0;
/// Default material for solid voxels.
pub const MATERIAL_DEFAULT: MaterialId = 1;

/// Chunk size with 1-voxel padding (64).
/// The padding allows neighbor lookups without bounds checks.
pub const CS_P: usize = 64;
/// Usable chunk size (62).
pub const CS: usize = 62;
/// Slice size (CS_P × CS_P = 4096).
pub const CS_P2: usize = CS_P * CS_P;
/// Total voxels (CS_P³ = 262144).
pub const CS_P3: usize = CS_P * CS_P * CS_P;

/// Face direction indices.
pub const FACE_POS_Y: usize = 0;
pub const FACE_NEG_Y: usize = 1;
pub const FACE_POS_X: usize = 2;
pub const FACE_NEG_X: usize = 3;
pub const FACE_POS_Z: usize = 4;
pub const FACE_NEG_Z: usize = 5;

/// Normal vectors for each face direction.
pub const FACE_NORMALS: [[f32; 3]; 6] = [
    [0.0, 1.0, 0.0],   // +Y
    [0.0, -1.0, 0.0],  // -Y
    [1.0, 0.0, 0.0],   // +X
    [-1.0, 0.0, 0.0],  // -X
    [0.0, 0.0, 1.0],   // +Z
    [0.0, 0.0, -1.0],  // -Z
];

/// Binary representation of a voxel chunk.
///
/// Uses column-based bitmasks for efficient face culling.
/// The 1-voxel padding allows neighbor lookups without bounds checks.
///
/// Memory layout:
/// - `opaque_mask[x * CS_P + z]` contains 64 bits representing the Y column at (x, z).
/// - `materials` uses a palette + bitpacked indices for per-voxel materials.
#[derive(Clone)]
pub struct BinaryChunk {
    /// Opaque mask: one bit per voxel, organized as Y columns.
    /// `opaque_mask[x * CS_P + z]` contains 64 bits for Y positions.
    pub opaque_mask: [u64; CS_P2],

    /// Palette-based materials for all voxels in the chunk.
    pub materials: crate::chunk::palette_materials::PaletteMaterials,
}

impl BinaryChunk {
    /// Create a new empty chunk.
    ///
    /// WARNING: This allocates ~32KB on the stack for the opaque mask.
    /// For WASM or stack-constrained environments, use `new_boxed()`.
    pub fn new() -> Self {
        Self {
            opaque_mask: [0u64; CS_P2],
            materials: crate::chunk::palette_materials::PaletteMaterials::new(),
        }
    }

    /// Create a new empty chunk on the heap.
    ///
    /// This is the preferred method for WASM and other stack-constrained
    /// environments.
    pub fn new_boxed() -> Box<Self> {
        Box::new(Self::new())
    }

    /// Set a voxel as solid with the given material.
    ///
    /// # Panics
    /// Debug panics if coordinates are out of bounds.
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, z: usize, material: MaterialId) {
        debug_assert!(x < CS_P && y < CS_P && z < CS_P, "Coordinates out of bounds");
        let column_idx = x * CS_P + z;
        self.opaque_mask[column_idx] |= 1u64 << y;
        self.materials.set_material(x, y, z, material);
    }

    /// Clear a voxel (make it empty).
    #[inline]
    pub fn clear(&mut self, x: usize, y: usize, z: usize) {
        debug_assert!(x < CS_P && y < CS_P && z < CS_P, "Coordinates out of bounds");
        let column_idx = x * CS_P + z;
        self.opaque_mask[column_idx] &= !(1u64 << y);
        self.materials.set_material(x, y, z, MATERIAL_EMPTY);
    }

    /// Check if a voxel is solid.
    #[inline]
    pub fn is_solid(&self, x: usize, y: usize, z: usize) -> bool {
        let column_idx = x * CS_P + z;
        (self.opaque_mask[column_idx] >> y) & 1 != 0
    }

    /// Get the material at a position (only valid if solid).
    #[inline]
    pub fn get_material(&self, x: usize, y: usize, z: usize) -> MaterialId {
        self.materials.get_material(x, y, z)
    }

    /// Count total solid voxels in the chunk.
    pub fn solid_count(&self) -> usize {
        self.opaque_mask.iter().map(|col| col.count_ones() as usize).sum()
    }

    /// Check if chunk is completely empty.
    pub fn is_empty(&self) -> bool {
        self.opaque_mask.iter().all(|&col| col == 0)
    }
}

impl Default for BinaryChunk {
    fn default() -> Self {
        Self::new()
    }
}

/// Face masks for all 6 directions after culling.
///
/// A face mask indicates which voxels have visible faces in a given direction.
#[derive(Clone)]
pub struct FaceMasks {
    /// `masks[face * CS_P2 + x * CS_P + z]` = visible faces in Y column.
    pub masks: [u64; 6 * CS_P2],
}

impl FaceMasks {
    /// Create empty face masks.
    pub fn new() -> Self {
        Self { masks: [0u64; 6 * CS_P2] }
    }

    /// Get the face mask for a column.
    #[inline]
    pub fn get(&self, face: usize, x: usize, z: usize) -> u64 {
        self.masks[face * CS_P2 + x * CS_P + z]
    }

    /// Set the face mask for a column.
    #[inline]
    pub fn set(&mut self, face: usize, x: usize, z: usize, mask: u64) {
        self.masks[face * CS_P2 + x * CS_P + z] = mask;
    }

    /// Get mutable reference to face mask.
    #[inline]
    pub fn get_mut(&mut self, face: usize, x: usize, z: usize) -> &mut u64 {
        &mut self.masks[face * CS_P2 + x * CS_P + z]
    }

    /// Count total visible faces.
    pub fn total_faces(&self) -> usize {
        self.masks.iter().map(|m| m.count_ones() as usize).sum()
    }

    /// Clear all masks.
    pub fn clear(&mut self) {
        self.masks.fill(0);
    }
}

impl Default for FaceMasks {
    fn default() -> Self {
        Self::new()
    }
}

/// Output mesh data ready for GPU buffers.
///
/// Contains vertex positions, normals, indices, and optionally UVs and material IDs.
#[derive(Default, Clone)]
pub struct MeshOutput {
    /// Vertex positions (3 floats per vertex).
    pub positions: Vec<f32>,
    /// Vertex normals (3 floats per vertex).
    pub normals: Vec<f32>,
    /// Triangle indices (3 indices per triangle).
    pub indices: Vec<u32>,
    /// UV coordinates (2 floats per vertex, optional).
    /// Tiled appropriately for merged quads (4x3 quad = 4x3 UV tiles).
    pub uvs: Vec<f32>,
    /// Per-vertex material IDs (optional, for shader lookup).
    /// All 4 vertices of a quad share the same material.
    pub material_ids: Vec<MaterialId>,
}

impl MeshOutput {
    /// Number of vertices in the mesh.
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// Number of triangles in the mesh.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Check if UVs are present.
    pub fn has_uvs(&self) -> bool {
        !self.uvs.is_empty()
    }

    /// Check if material IDs are present.
    pub fn has_material_ids(&self) -> bool {
        !self.material_ids.is_empty()
    }

    /// Clear all mesh data.
    pub fn clear(&mut self) {
        self.positions.clear();
        self.normals.clear();
        self.indices.clear();
        self.uvs.clear();
        self.material_ids.clear();
    }

    /// Pre-allocate capacity for estimated quad count.
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

/// Pack quad data into 64 bits for efficient storage.
///
/// # Layout
/// - Bits 0-5: X position (0-63)
/// - Bits 6-11: Y position (0-63)
/// - Bits 12-17: Z position (0-63)
/// - Bits 18-23: Width (1-64, stored as 0-63)
/// - Bits 24-29: Height (1-64, stored as 0-63)
/// - Bits 30-31: Reserved
/// - Bits 32-47: Material ID (0-65535)
/// - Bits 48-63: Reserved
#[inline]
pub fn pack_quad(x: u32, y: u32, z: u32, w: u32, h: u32, material: MaterialId) -> u64 {
    ((material as u64) << 32)
        | ((h as u64 & 0x3F) << 24)
        | ((w as u64 & 0x3F) << 18)
        | ((z as u64 & 0x3F) << 12)
        | ((y as u64 & 0x3F) << 6)
        | (x as u64 & 0x3F)
}

/// Unpack quad data from 64 bits.
///
/// Returns (x, y, z, width, height, material).
#[inline]
pub fn unpack_quad(quad: u64) -> (u32, u32, u32, u32, u32, MaterialId) {
    let x = (quad & 0x3F) as u32;
    let y = ((quad >> 6) & 0x3F) as u32;
    let z = ((quad >> 12) & 0x3F) as u32;
    let w = ((quad >> 18) & 0x3F) as u32;
    let h = ((quad >> 24) & 0x3F) as u32;
    let material = ((quad >> 32) & 0xFFFF) as MaterialId;
    (x, y, z, w, h, material)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_roundtrip() {
        let quad = pack_quad(10, 20, 30, 5, 8, 1234);
        let (x, y, z, w, h, mat) = unpack_quad(quad);
        assert_eq!((x, y, z, w, h, mat), (10, 20, 30, 5, 8, 1234));
    }

    #[test]
    fn pack_unpack_max_values() {
        let quad = pack_quad(63, 63, 63, 63, 63, 65535);
        let (x, y, z, w, h, mat) = unpack_quad(quad);
        assert_eq!((x, y, z, w, h, mat), (63, 63, 63, 63, 63, 65535));
    }

    #[test]
    fn pack_unpack_zeros() {
        let quad = pack_quad(0, 0, 0, 0, 0, 0);
        let (x, y, z, w, h, mat) = unpack_quad(quad);
        assert_eq!((x, y, z, w, h, mat), (0, 0, 0, 0, 0, 0));
    }

    #[test]
    fn chunk_set_get() {
        let mut chunk = BinaryChunk::new();
        chunk.set(10, 20, 30, 42);

        assert!(chunk.is_solid(10, 20, 30));
        assert_eq!(chunk.get_material(10, 20, 30), 42);
        assert!(!chunk.is_solid(10, 20, 31));
    }

    #[test]
    fn chunk_clear() {
        let mut chunk = BinaryChunk::new();
        chunk.set(10, 20, 30, 42);
        assert!(chunk.is_solid(10, 20, 30));

        chunk.clear(10, 20, 30);
        assert!(!chunk.is_solid(10, 20, 30));
        assert_eq!(chunk.get_material(10, 20, 30), MATERIAL_EMPTY);
    }

    #[test]
    fn chunk_solid_count() {
        let mut chunk = BinaryChunk::new();
        assert_eq!(chunk.solid_count(), 0);

        chunk.set(10, 20, 30, 1);
        chunk.set(11, 20, 30, 1);
        chunk.set(12, 20, 30, 1);
        assert_eq!(chunk.solid_count(), 3);
    }

    #[test]
    fn chunk_is_empty() {
        let chunk = BinaryChunk::new();
        assert!(chunk.is_empty());

        let mut chunk2 = BinaryChunk::new();
        chunk2.set(10, 20, 30, 1);
        assert!(!chunk2.is_empty());
    }

    #[test]
    fn face_masks_get_set() {
        let mut masks = FaceMasks::new();
        masks.set(FACE_POS_Y, 10, 20, 0xFFFF);

        assert_eq!(masks.get(FACE_POS_Y, 10, 20), 0xFFFF);
        assert_eq!(masks.get(FACE_POS_Y, 10, 21), 0);
    }

    #[test]
    fn mesh_output_counts() {
        let mut mesh = MeshOutput::default();
        assert_eq!(mesh.vertex_count(), 0);
        assert_eq!(mesh.triangle_count(), 0);

        // Add one quad (4 vertices, 2 triangles)
        mesh.positions.extend_from_slice(&[0.0; 12]); // 4 * 3
        mesh.normals.extend_from_slice(&[0.0; 12]);
        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);

        assert_eq!(mesh.vertex_count(), 4);
        assert_eq!(mesh.triangle_count(), 2);
    }

    #[test]
    fn chunk_constants() {
        assert_eq!(CS_P, 64);
        assert_eq!(CS, 62);
        assert_eq!(CS_P2, 64 * 64);
        assert_eq!(CS_P3, 64 * 64 * 64);
    }
}
