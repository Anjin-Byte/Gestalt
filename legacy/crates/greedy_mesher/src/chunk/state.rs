//! Chunk state machine and boundary tracking.

/// Lifecycle state of a chunk's mesh.
///
/// Tracks whether a chunk's mesh is up-to-date with its voxel data
/// and manages the async meshing workflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkState {
    /// Mesh is up-to-date with voxel data.
    Clean,

    /// Voxel data changed; mesh needs rebuild.
    Dirty,

    /// Currently being meshed (async job in progress).
    ///
    /// The `data_version` field records the version when meshing started,
    /// used to detect if the chunk was modified during meshing.
    Meshing {
        /// Version of voxel data when meshing started.
        data_version: u64,
    },

    /// New mesh ready; waiting to swap into render.
    ///
    /// The mesh was built from voxel data at `data_version`.
    ReadyToSwap {
        /// Version of voxel data the mesh was built from.
        data_version: u64,
    },
}

impl Default for ChunkState {
    fn default() -> Self {
        ChunkState::Dirty
    }
}

impl ChunkState {
    /// Check if this chunk needs a mesh rebuild.
    pub fn needs_rebuild(&self) -> bool {
        matches!(self, ChunkState::Dirty)
    }

    /// Check if this chunk is currently being meshed.
    pub fn is_meshing(&self) -> bool {
        matches!(self, ChunkState::Meshing { .. })
    }

    /// Check if this chunk has a pending mesh ready to swap.
    pub fn has_pending_mesh(&self) -> bool {
        matches!(self, ChunkState::ReadyToSwap { .. })
    }

    /// Check if this chunk's mesh is up-to-date.
    pub fn is_clean(&self) -> bool {
        matches!(self, ChunkState::Clean)
    }
}

/// Flags indicating which chunk boundaries a voxel touches.
///
/// Used to determine which neighbor chunks need to be marked dirty
/// when a voxel on the boundary is modified.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BoundaryFlags {
    /// Voxel is on the -X face (local x == 0).
    pub neg_x: bool,
    /// Voxel is on the +X face (local x == SIZE - 1).
    pub pos_x: bool,
    /// Voxel is on the -Y face (local y == 0).
    pub neg_y: bool,
    /// Voxel is on the +Y face (local y == SIZE - 1).
    pub pos_y: bool,
    /// Voxel is on the -Z face (local z == 0).
    pub neg_z: bool,
    /// Voxel is on the +Z face (local z == SIZE - 1).
    pub pos_z: bool,
}

impl BoundaryFlags {
    /// Check if any boundary flag is set.
    pub fn any(&self) -> bool {
        self.neg_x || self.pos_x || self.neg_y || self.pos_y || self.neg_z || self.pos_z
    }

    /// Count how many boundaries this touches.
    pub fn count(&self) -> usize {
        let mut count = 0;
        if self.neg_x { count += 1; }
        if self.pos_x { count += 1; }
        if self.neg_y { count += 1; }
        if self.pos_y { count += 1; }
        if self.neg_z { count += 1; }
        if self.pos_z { count += 1; }
        count
    }

    /// Get neighbor chunk offsets that need to be marked dirty.
    ///
    /// Returns offsets as [dx, dy, dz] for each boundary that is set.
    pub fn affected_neighbors(&self) -> Vec<[i32; 3]> {
        let mut neighbors = Vec::with_capacity(6);
        if self.neg_x { neighbors.push([-1, 0, 0]); }
        if self.pos_x { neighbors.push([1, 0, 0]); }
        if self.neg_y { neighbors.push([0, -1, 0]); }
        if self.pos_y { neighbors.push([0, 1, 0]); }
        if self.neg_z { neighbors.push([0, 0, -1]); }
        if self.pos_z { neighbors.push([0, 0, 1]); }
        neighbors
    }

    /// Merge another set of boundary flags into this one (OR operation).
    pub fn merge(&mut self, other: BoundaryFlags) {
        self.neg_x |= other.neg_x;
        self.pos_x |= other.pos_x;
        self.neg_y |= other.neg_y;
        self.pos_y |= other.pos_y;
        self.neg_z |= other.neg_z;
        self.pos_z |= other.pos_z;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_state_default_is_dirty() {
        assert_eq!(ChunkState::default(), ChunkState::Dirty);
    }

    #[test]
    fn chunk_state_needs_rebuild() {
        assert!(ChunkState::Dirty.needs_rebuild());
        assert!(!ChunkState::Clean.needs_rebuild());
        assert!(!ChunkState::Meshing { data_version: 1 }.needs_rebuild());
        assert!(!ChunkState::ReadyToSwap { data_version: 1 }.needs_rebuild());
    }

    #[test]
    fn chunk_state_is_meshing() {
        assert!(ChunkState::Meshing { data_version: 1 }.is_meshing());
        assert!(!ChunkState::Dirty.is_meshing());
        assert!(!ChunkState::Clean.is_meshing());
    }

    #[test]
    fn chunk_state_has_pending_mesh() {
        assert!(ChunkState::ReadyToSwap { data_version: 1 }.has_pending_mesh());
        assert!(!ChunkState::Dirty.has_pending_mesh());
        assert!(!ChunkState::Clean.has_pending_mesh());
    }

    #[test]
    fn boundary_flags_default_is_none() {
        let flags = BoundaryFlags::default();
        assert!(!flags.any());
        assert_eq!(flags.count(), 0);
    }

    #[test]
    fn boundary_flags_any() {
        let mut flags = BoundaryFlags::default();
        assert!(!flags.any());

        flags.neg_x = true;
        assert!(flags.any());
    }

    #[test]
    fn boundary_flags_count() {
        let flags = BoundaryFlags {
            neg_x: true,
            pos_x: false,
            neg_y: true,
            pos_y: false,
            neg_z: false,
            pos_z: true,
        };
        assert_eq!(flags.count(), 3);
    }

    #[test]
    fn boundary_flags_affected_neighbors() {
        let flags = BoundaryFlags {
            neg_x: true,
            pos_x: false,
            neg_y: false,
            pos_y: true,
            neg_z: false,
            pos_z: false,
        };

        let neighbors = flags.affected_neighbors();
        assert_eq!(neighbors.len(), 2);
        assert!(neighbors.contains(&[-1, 0, 0]));
        assert!(neighbors.contains(&[0, 1, 0]));
    }

    #[test]
    fn boundary_flags_merge() {
        let mut flags1 = BoundaryFlags {
            neg_x: true,
            pos_x: false,
            neg_y: false,
            pos_y: false,
            neg_z: false,
            pos_z: false,
        };

        let flags2 = BoundaryFlags {
            neg_x: false,
            pos_x: true,
            neg_y: false,
            pos_y: true,
            neg_z: false,
            pos_z: false,
        };

        flags1.merge(flags2);

        assert!(flags1.neg_x);
        assert!(flags1.pos_x);
        assert!(!flags1.neg_y);
        assert!(flags1.pos_y);
        assert!(!flags1.neg_z);
        assert!(!flags1.pos_z);
    }
}
