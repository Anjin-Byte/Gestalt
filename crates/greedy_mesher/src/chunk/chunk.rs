//! Chunk data structure wrapping BinaryChunk with state management.

use crate::core::{BinaryChunk, MaterialId, MeshOutput, CS, MATERIAL_EMPTY};
use super::coord::ChunkCoord;
use super::state::{ChunkState, BoundaryFlags};

/// Mesh data for a single chunk.
///
/// Contains the generated geometry and metadata for rendering.
#[derive(Clone, Debug)]
pub struct ChunkMesh {
    /// Vertex positions (flattened xyz triplets).
    pub positions: Vec<f32>,
    /// Vertex normals (flattened xyz triplets).
    pub normals: Vec<f32>,
    /// Triangle indices.
    pub indices: Vec<u32>,
    /// UV coordinates (flattened uv pairs).
    pub uvs: Vec<f32>,
    /// Per-vertex material IDs.
    pub material_ids: Vec<u16>,

    /// Version of voxel data this mesh was built from.
    pub data_version: u64,

    /// Number of triangles in the mesh.
    pub triangle_count: usize,
    /// Number of vertices in the mesh.
    pub vertex_count: usize,
}

impl ChunkMesh {
    /// Create an empty mesh placeholder.
    pub fn empty() -> Self {
        Self {
            positions: Vec::new(),
            normals: Vec::new(),
            indices: Vec::new(),
            uvs: Vec::new(),
            material_ids: Vec::new(),
            data_version: 0,
            triangle_count: 0,
            vertex_count: 0,
        }
    }

    /// Create a ChunkMesh from a MeshOutput.
    pub fn from_mesh_output(output: MeshOutput, data_version: u64) -> Self {
        let vertex_count = output.positions.len() / 3;
        let triangle_count = output.indices.len() / 3;

        Self {
            positions: output.positions,
            normals: output.normals,
            indices: output.indices,
            uvs: output.uvs,
            material_ids: output.material_ids,
            data_version,
            triangle_count,
            vertex_count,
        }
    }

    /// Check if the mesh is empty.
    pub fn is_empty(&self) -> bool {
        self.vertex_count == 0
    }

    /// Get approximate memory usage in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.positions.len() * 4 +
        self.normals.len() * 4 +
        self.indices.len() * 4 +
        self.uvs.len() * 4 +
        self.material_ids.len() * 2
    }
}

/// Complete chunk data structure.
///
/// Combines voxel storage (BinaryChunk), mesh state, and cached mesh data.
/// The voxel storage is boxed to avoid stack overflow (BinaryChunk is ~544KB).
#[derive(Clone)]
pub struct Chunk {
    /// Chunk coordinate in chunk-space.
    pub coord: ChunkCoord,

    /// Current lifecycle state.
    pub state: ChunkState,

    /// Monotonically increasing version, incremented on any voxel edit.
    pub data_version: u64,

    /// Voxel storage with binary masks and material IDs (boxed due to large size).
    pub voxels: Box<BinaryChunk>,

    /// Cached mesh data (if state is Clean or has been meshed).
    pub mesh: Option<ChunkMesh>,

    /// Pending mesh from async job (if state is ReadyToSwap).
    pub pending_mesh: Option<ChunkMesh>,
}

impl Chunk {
    /// Usable chunk size (excluding padding).
    pub const SIZE: u32 = CS as u32;

    /// Create a new empty chunk at the given coordinate.
    ///
    /// New chunks start in Dirty state since they need an initial mesh.
    pub fn new(coord: ChunkCoord) -> Self {
        Self {
            coord,
            state: ChunkState::Dirty,
            data_version: 0,
            voxels: Box::new(BinaryChunk::new()),
            mesh: None,
            pending_mesh: None,
        }
    }

    /// Get material at local coordinates (0..SIZE).
    ///
    /// Coordinates are in local chunk space, not world space.
    /// Returns MATERIAL_EMPTY for out-of-bounds coordinates.
    pub fn get_voxel(&self, x: u32, y: u32, z: u32) -> MaterialId {
        if x >= Self::SIZE || y >= Self::SIZE || z >= Self::SIZE {
            return MATERIAL_EMPTY;
        }
        // Add 1 for padding offset in BinaryChunk
        self.voxels.get_material(x as usize + 1, y as usize + 1, z as usize + 1)
    }

    /// Set voxel at local coordinates.
    ///
    /// Automatically increments data_version.
    pub fn set_voxel(&mut self, x: u32, y: u32, z: u32, material: MaterialId) {
        if x >= Self::SIZE || y >= Self::SIZE || z >= Self::SIZE {
            return;
        }
        // Add 1 for padding offset in BinaryChunk
        self.voxels.set(x as usize + 1, y as usize + 1, z as usize + 1, material);
        self.data_version += 1;
    }

    /// Set voxel without incrementing version (for batch operations).
    ///
    /// Caller must manually increment data_version after batch is complete.
    pub fn set_voxel_raw(&mut self, x: u32, y: u32, z: u32, material: MaterialId) {
        if x >= Self::SIZE || y >= Self::SIZE || z >= Self::SIZE {
            return;
        }
        self.voxels.set(x as usize + 1, y as usize + 1, z as usize + 1, material);
    }

    /// Increment data version (call after batch set_voxel_raw operations).
    pub fn increment_version(&mut self) {
        self.data_version += 1;
    }

    /// Check if local coordinate is on chunk boundary.
    pub fn is_on_boundary(&self, x: u32, y: u32, z: u32) -> BoundaryFlags {
        BoundaryFlags {
            neg_x: x == 0,
            pos_x: x == Self::SIZE - 1,
            neg_y: y == 0,
            pos_y: y == Self::SIZE - 1,
            neg_z: z == 0,
            pos_z: z == Self::SIZE - 1,
        }
    }

    /// Check if this chunk contains any solid voxels.
    pub fn is_empty(&self) -> bool {
        self.voxels.opaque_mask.iter().all(|&col| col == 0)
    }

    /// Count the number of solid voxels in this chunk.
    pub fn solid_count(&self) -> usize {
        self.voxels.opaque_mask.iter()
            .map(|col| col.count_ones() as usize)
            .sum()
    }

    /// Get the fill ratio (solid voxels / total capacity).
    pub fn fill_ratio(&self) -> f32 {
        let total = (Self::SIZE * Self::SIZE * Self::SIZE) as f32;
        self.solid_count() as f32 / total
    }

    /// Mark this chunk as dirty (needs rebuild).
    pub fn mark_dirty(&mut self) {
        self.state = ChunkState::Dirty;
    }

    /// Mark this chunk as meshing with current data version.
    pub fn mark_meshing(&mut self) {
        self.state = ChunkState::Meshing {
            data_version: self.data_version,
        };
    }

    /// Mark this chunk as having a pending mesh ready.
    pub fn mark_ready_to_swap(&mut self, mesh: ChunkMesh) {
        self.pending_mesh = Some(mesh);
        self.state = ChunkState::ReadyToSwap {
            data_version: self.data_version,
        };
    }

    /// Attempt to swap pending mesh into active slot.
    ///
    /// Returns true if swap succeeded, false if version mismatch.
    pub fn try_swap_mesh(&mut self) -> bool {
        if let ChunkState::ReadyToSwap { data_version } = self.state {
            if data_version == self.data_version {
                // Version matches - swap mesh
                if let Some(pending) = self.pending_mesh.take() {
                    self.mesh = Some(pending);
                    self.state = ChunkState::Clean;
                    return true;
                }
            } else {
                // Version mismatch - discard pending and mark dirty
                self.pending_mesh = None;
                self.state = ChunkState::Dirty;
            }
        }
        false
    }

    /// Get reference to the active mesh, if any.
    pub fn get_mesh(&self) -> Option<&ChunkMesh> {
        self.mesh.as_ref()
    }

    /// Clear all voxels and reset state.
    pub fn clear(&mut self) {
        self.voxels = Box::new(BinaryChunk::new());
        self.data_version += 1;
        self.state = ChunkState::Dirty;
        self.pending_mesh = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::MATERIAL_DEFAULT;

    #[test]
    fn new_chunk_is_dirty() {
        let chunk = Chunk::new(ChunkCoord::ZERO);
        assert_eq!(chunk.state, ChunkState::Dirty);
        assert_eq!(chunk.data_version, 0);
        assert!(chunk.mesh.is_none());
    }

    #[test]
    fn new_chunk_is_empty() {
        let chunk = Chunk::new(ChunkCoord::ZERO);
        assert!(chunk.is_empty());
        assert_eq!(chunk.solid_count(), 0);
    }

    #[test]
    fn set_voxel_increments_version() {
        let mut chunk = Chunk::new(ChunkCoord::ZERO);
        assert_eq!(chunk.data_version, 0);

        chunk.set_voxel(0, 0, 0, MATERIAL_DEFAULT);
        assert_eq!(chunk.data_version, 1);

        chunk.set_voxel(1, 1, 1, MATERIAL_DEFAULT);
        assert_eq!(chunk.data_version, 2);
    }

    #[test]
    fn get_set_voxel_roundtrip() {
        let mut chunk = Chunk::new(ChunkCoord::ZERO);

        chunk.set_voxel(10, 20, 30, 42);
        assert_eq!(chunk.get_voxel(10, 20, 30), 42);

        // Other positions still empty
        assert_eq!(chunk.get_voxel(0, 0, 0), MATERIAL_EMPTY);
    }

    #[test]
    fn solid_count_and_fill_ratio() {
        let mut chunk = Chunk::new(ChunkCoord::ZERO);

        // Set 100 voxels
        for i in 0..10 {
            for j in 0..10 {
                chunk.set_voxel_raw(i, j, 0, MATERIAL_DEFAULT);
            }
        }
        chunk.increment_version();

        assert_eq!(chunk.solid_count(), 100);
        assert!(!chunk.is_empty());

        let expected_ratio = 100.0 / (62.0 * 62.0 * 62.0);
        assert!((chunk.fill_ratio() - expected_ratio).abs() < 0.0001);
    }

    #[test]
    fn boundary_detection() {
        let chunk = Chunk::new(ChunkCoord::ZERO);

        // Interior voxel
        let flags = chunk.is_on_boundary(30, 30, 30);
        assert!(!flags.any());

        // Corner voxel (on 3 boundaries)
        let flags = chunk.is_on_boundary(0, 0, 0);
        assert!(flags.neg_x && flags.neg_y && flags.neg_z);
        assert!(!flags.pos_x && !flags.pos_y && !flags.pos_z);
        assert_eq!(flags.count(), 3);

        // Edge voxel
        let flags = chunk.is_on_boundary(61, 61, 30);
        assert!(flags.pos_x && flags.pos_y);
        assert!(!flags.neg_x && !flags.neg_y && !flags.neg_z && !flags.pos_z);
        assert_eq!(flags.count(), 2);
    }

    #[test]
    fn mesh_swap_version_match() {
        let mut chunk = Chunk::new(ChunkCoord::ZERO);
        chunk.data_version = 5;

        // Create a pending mesh with matching version
        let mesh = ChunkMesh::empty();
        chunk.pending_mesh = Some(mesh);
        chunk.state = ChunkState::ReadyToSwap { data_version: 5 };

        assert!(chunk.try_swap_mesh());
        assert_eq!(chunk.state, ChunkState::Clean);
        assert!(chunk.mesh.is_some());
        assert!(chunk.pending_mesh.is_none());
    }

    #[test]
    fn mesh_swap_version_mismatch() {
        let mut chunk = Chunk::new(ChunkCoord::ZERO);
        chunk.data_version = 10; // Chunk was edited

        // Create a pending mesh with old version
        let mesh = ChunkMesh::empty();
        chunk.pending_mesh = Some(mesh);
        chunk.state = ChunkState::ReadyToSwap { data_version: 5 };

        assert!(!chunk.try_swap_mesh());
        assert_eq!(chunk.state, ChunkState::Dirty);
        assert!(chunk.mesh.is_none());
        assert!(chunk.pending_mesh.is_none());
    }

    #[test]
    fn clear_resets_chunk() {
        let mut chunk = Chunk::new(ChunkCoord::ZERO);
        chunk.set_voxel(10, 10, 10, MATERIAL_DEFAULT);
        chunk.mesh = Some(ChunkMesh::empty());
        chunk.state = ChunkState::Clean;

        let old_version = chunk.data_version;
        chunk.clear();

        assert!(chunk.is_empty());
        assert_eq!(chunk.state, ChunkState::Dirty);
        assert!(chunk.data_version > old_version);
    }
}
