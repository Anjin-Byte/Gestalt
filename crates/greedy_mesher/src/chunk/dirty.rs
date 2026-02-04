//! Dirty tracking for chunk mesh rebuilds.

use std::collections::HashSet;
use super::coord::ChunkCoord;
use super::state::BoundaryFlags;

/// Tracks which chunks need mesh rebuilds.
///
/// Uses a HashSet for automatic deduplication - multiple edits to
/// the same chunk only result in one rebuild.
#[derive(Clone, Debug, Default)]
pub struct DirtyTracker {
    /// Set of dirty chunk coordinates (deduped by nature of HashSet).
    dirty_chunks: HashSet<ChunkCoord>,
}

impl DirtyTracker {
    /// Create a new empty dirty tracker.
    pub fn new() -> Self {
        Self {
            dirty_chunks: HashSet::new(),
        }
    }

    /// Mark a single chunk as dirty.
    ///
    /// Returns true if the chunk was not already dirty.
    pub fn mark_dirty(&mut self, coord: ChunkCoord) -> bool {
        self.dirty_chunks.insert(coord)
    }

    /// Mark chunk and boundary-affected neighbors as dirty.
    ///
    /// When a voxel on a chunk boundary is modified, the adjacent chunk
    /// also needs to rebuild its mesh since the face culling may change.
    pub fn mark_dirty_with_neighbors(&mut self, coord: ChunkCoord, boundary: BoundaryFlags) {
        self.dirty_chunks.insert(coord);

        for offset in boundary.affected_neighbors() {
            let neighbor = ChunkCoord {
                x: coord.x + offset[0],
                y: coord.y + offset[1],
                z: coord.z + offset[2],
            };
            self.dirty_chunks.insert(neighbor);
        }
    }

    /// Remove a chunk from the dirty set.
    ///
    /// Call this when a chunk starts meshing.
    pub fn unmark(&mut self, coord: ChunkCoord) -> bool {
        self.dirty_chunks.remove(&coord)
    }

    /// Check if a specific chunk is dirty.
    pub fn is_dirty(&self, coord: ChunkCoord) -> bool {
        self.dirty_chunks.contains(&coord)
    }

    /// Take all dirty chunks (clears the set).
    ///
    /// Returns ownership of the dirty set for processing.
    pub fn take_dirty(&mut self) -> HashSet<ChunkCoord> {
        std::mem::take(&mut self.dirty_chunks)
    }

    /// Drain dirty chunks as an iterator.
    ///
    /// More efficient than take_dirty when iterating immediately.
    pub fn drain(&mut self) -> impl Iterator<Item = ChunkCoord> + '_ {
        self.dirty_chunks.drain()
    }

    /// Check if any chunks are dirty.
    pub fn has_dirty(&self) -> bool {
        !self.dirty_chunks.is_empty()
    }

    /// Number of dirty chunks.
    pub fn dirty_count(&self) -> usize {
        self.dirty_chunks.len()
    }

    /// Clear all dirty markers.
    pub fn clear(&mut self) {
        self.dirty_chunks.clear();
    }

    /// Get an iterator over dirty chunk coordinates.
    pub fn iter(&self) -> impl Iterator<Item = &ChunkCoord> {
        self.dirty_chunks.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_tracker_is_empty() {
        let tracker = DirtyTracker::new();
        assert!(!tracker.has_dirty());
        assert_eq!(tracker.dirty_count(), 0);
    }

    #[test]
    fn mark_dirty_single() {
        let mut tracker = DirtyTracker::new();
        let coord = ChunkCoord::new(1, 2, 3);

        assert!(tracker.mark_dirty(coord)); // First mark returns true
        assert!(!tracker.mark_dirty(coord)); // Second mark returns false (already dirty)

        assert!(tracker.has_dirty());
        assert_eq!(tracker.dirty_count(), 1);
        assert!(tracker.is_dirty(coord));
    }

    #[test]
    fn mark_dirty_multiple() {
        let mut tracker = DirtyTracker::new();

        tracker.mark_dirty(ChunkCoord::new(0, 0, 0));
        tracker.mark_dirty(ChunkCoord::new(1, 0, 0));
        tracker.mark_dirty(ChunkCoord::new(0, 1, 0));

        assert_eq!(tracker.dirty_count(), 3);
    }

    #[test]
    fn deduplication() {
        let mut tracker = DirtyTracker::new();
        let coord = ChunkCoord::new(5, 5, 5);

        // Mark same chunk multiple times
        for _ in 0..10 {
            tracker.mark_dirty(coord);
        }

        // Should only be one entry
        assert_eq!(tracker.dirty_count(), 1);
    }

    #[test]
    fn mark_dirty_with_neighbors_interior() {
        let mut tracker = DirtyTracker::new();
        let coord = ChunkCoord::new(5, 5, 5);

        // Interior voxel - no neighbors affected
        let boundary = BoundaryFlags::default();
        tracker.mark_dirty_with_neighbors(coord, boundary);

        assert_eq!(tracker.dirty_count(), 1);
    }

    #[test]
    fn mark_dirty_with_neighbors_boundary() {
        let mut tracker = DirtyTracker::new();
        let coord = ChunkCoord::new(5, 5, 5);

        // Voxel on +X boundary
        let boundary = BoundaryFlags {
            pos_x: true,
            ..Default::default()
        };
        tracker.mark_dirty_with_neighbors(coord, boundary);

        assert_eq!(tracker.dirty_count(), 2);
        assert!(tracker.is_dirty(coord));
        assert!(tracker.is_dirty(ChunkCoord::new(6, 5, 5))); // +X neighbor
    }

    #[test]
    fn mark_dirty_with_neighbors_corner() {
        let mut tracker = DirtyTracker::new();
        let coord = ChunkCoord::new(5, 5, 5);

        // Corner voxel touching 3 boundaries
        let boundary = BoundaryFlags {
            neg_x: true,
            neg_y: true,
            neg_z: true,
            ..Default::default()
        };
        tracker.mark_dirty_with_neighbors(coord, boundary);

        assert_eq!(tracker.dirty_count(), 4); // Self + 3 neighbors
        assert!(tracker.is_dirty(coord));
        assert!(tracker.is_dirty(ChunkCoord::new(4, 5, 5))); // -X
        assert!(tracker.is_dirty(ChunkCoord::new(5, 4, 5))); // -Y
        assert!(tracker.is_dirty(ChunkCoord::new(5, 5, 4))); // -Z
    }

    #[test]
    fn take_dirty_clears_set() {
        let mut tracker = DirtyTracker::new();
        tracker.mark_dirty(ChunkCoord::new(1, 1, 1));
        tracker.mark_dirty(ChunkCoord::new(2, 2, 2));

        let taken = tracker.take_dirty();

        assert_eq!(taken.len(), 2);
        assert!(!tracker.has_dirty());
        assert_eq!(tracker.dirty_count(), 0);
    }

    #[test]
    fn unmark() {
        let mut tracker = DirtyTracker::new();
        let coord = ChunkCoord::new(1, 1, 1);

        tracker.mark_dirty(coord);
        assert!(tracker.is_dirty(coord));

        assert!(tracker.unmark(coord));
        assert!(!tracker.is_dirty(coord));

        // Unmark non-existent returns false
        assert!(!tracker.unmark(coord));
    }

    #[test]
    fn clear() {
        let mut tracker = DirtyTracker::new();
        tracker.mark_dirty(ChunkCoord::new(1, 1, 1));
        tracker.mark_dirty(ChunkCoord::new(2, 2, 2));

        tracker.clear();

        assert!(!tracker.has_dirty());
        assert_eq!(tracker.dirty_count(), 0);
    }
}
