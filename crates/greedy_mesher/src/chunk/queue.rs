//! Priority queue for chunk mesh rebuilds.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};
use super::coord::ChunkCoord;

/// Rebuild request with priority.
///
/// Higher priority values are processed first (closer to camera = higher priority).
#[derive(Clone, Debug)]
pub struct RebuildRequest {
    /// Chunk coordinate to rebuild.
    pub coord: ChunkCoord,
    /// Priority value (higher = more urgent).
    pub priority: f32,
    /// Version of voxel data when request was created.
    pub data_version: u64,
}

impl PartialEq for RebuildRequest {
    fn eq(&self, other: &Self) -> bool {
        self.coord == other.coord
    }
}

impl Eq for RebuildRequest {}

impl PartialOrd for RebuildRequest {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RebuildRequest {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher priority first (reverse ordering for max-heap behavior)
        self.priority
            .partial_cmp(&other.priority)
            .unwrap_or(Ordering::Equal)
    }
}

/// Priority queue for chunk rebuilds.
///
/// Ensures closer chunks (higher priority) are rebuilt first.
/// Automatically deduplicates - each chunk can only be in the queue once.
#[derive(Debug, Default)]
pub struct RebuildQueue {
    /// Priority queue of rebuild requests.
    queue: BinaryHeap<RebuildRequest>,
    /// Track which chunks are already in queue (for deduplication).
    in_queue: HashSet<ChunkCoord>,
}

impl RebuildQueue {
    /// Create a new empty rebuild queue.
    pub fn new() -> Self {
        Self {
            queue: BinaryHeap::new(),
            in_queue: HashSet::new(),
        }
    }

    /// Create a queue with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            queue: BinaryHeap::with_capacity(capacity),
            in_queue: HashSet::with_capacity(capacity),
        }
    }

    /// Add chunk to rebuild queue with priority.
    ///
    /// If the chunk is already in the queue, this is a no-op.
    /// Returns true if the chunk was added, false if already present.
    pub fn enqueue(&mut self, coord: ChunkCoord, priority: f32, data_version: u64) -> bool {
        if self.in_queue.insert(coord) {
            self.queue.push(RebuildRequest {
                coord,
                priority,
                data_version,
            });
            true
        } else {
            false
        }
    }

    /// Pop highest-priority chunk.
    ///
    /// Returns None if the queue is empty.
    pub fn pop(&mut self) -> Option<RebuildRequest> {
        while let Some(request) = self.queue.pop() {
            // Check if still in queue (handles case where it was removed)
            if self.in_queue.remove(&request.coord) {
                return Some(request);
            }
        }
        None
    }

    /// Peek at the highest-priority request without removing it.
    pub fn peek(&self) -> Option<&RebuildRequest> {
        self.queue.peek()
    }

    /// Remove a specific chunk from the queue.
    ///
    /// Returns true if the chunk was in the queue.
    /// Note: This is O(1) for the set but the heap entry remains
    /// until it's popped (lazy removal).
    pub fn remove(&mut self, coord: ChunkCoord) -> bool {
        self.in_queue.remove(&coord)
    }

    /// Check if a chunk is in the queue.
    pub fn contains(&self, coord: ChunkCoord) -> bool {
        self.in_queue.contains(&coord)
    }

    /// Number of pending rebuilds.
    pub fn len(&self) -> usize {
        self.in_queue.len()
    }

    /// Check if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.in_queue.is_empty()
    }

    /// Clear all pending rebuilds.
    pub fn clear(&mut self) {
        self.queue.clear();
        self.in_queue.clear();
    }

    /// Update priority for a chunk if it's in the queue.
    ///
    /// This re-inserts the request with new priority (old one is lazily removed on pop).
    pub fn update_priority(&mut self, coord: ChunkCoord, priority: f32, data_version: u64) {
        if self.in_queue.contains(&coord) {
            // Add new entry with updated priority
            // Old entry will be skipped in pop() since coord won't be in in_queue
            self.queue.push(RebuildRequest {
                coord,
                priority,
                data_version,
            });
        }
    }

    /// Get iterator over chunks in queue (unordered).
    pub fn iter(&self) -> impl Iterator<Item = &ChunkCoord> {
        self.in_queue.iter()
    }
}

/// Calculate rebuild priority based on camera distance.
///
/// Returns a priority value where higher = more urgent.
/// Chunks closer to the camera have higher priority.
pub fn calculate_priority(chunk_center: [f32; 3], camera_pos: [f32; 3]) -> f32 {
    let dx = chunk_center[0] - camera_pos[0];
    let dy = chunk_center[1] - camera_pos[1];
    let dz = chunk_center[2] - camera_pos[2];
    let distance_sq = dx * dx + dy * dy + dz * dz;

    // Invert so closer = higher priority
    // Add small epsilon to avoid division by zero
    1.0 / (distance_sq + 0.001)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_queue_is_empty() {
        let queue = RebuildQueue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn enqueue_and_pop() {
        let mut queue = RebuildQueue::new();

        queue.enqueue(ChunkCoord::new(1, 1, 1), 1.0, 0);
        queue.enqueue(ChunkCoord::new(2, 2, 2), 2.0, 0);
        queue.enqueue(ChunkCoord::new(3, 3, 3), 0.5, 0);

        assert_eq!(queue.len(), 3);

        // Should pop in priority order (highest first)
        let req = queue.pop().unwrap();
        assert_eq!(req.coord, ChunkCoord::new(2, 2, 2));
        assert_eq!(req.priority, 2.0);

        let req = queue.pop().unwrap();
        assert_eq!(req.coord, ChunkCoord::new(1, 1, 1));

        let req = queue.pop().unwrap();
        assert_eq!(req.coord, ChunkCoord::new(3, 3, 3));

        assert!(queue.pop().is_none());
    }

    #[test]
    fn deduplication() {
        let mut queue = RebuildQueue::new();
        let coord = ChunkCoord::new(1, 1, 1);

        assert!(queue.enqueue(coord, 1.0, 0));
        assert!(!queue.enqueue(coord, 2.0, 0)); // Already in queue

        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn contains() {
        let mut queue = RebuildQueue::new();
        let coord = ChunkCoord::new(1, 1, 1);

        assert!(!queue.contains(coord));

        queue.enqueue(coord, 1.0, 0);
        assert!(queue.contains(coord));

        queue.pop();
        assert!(!queue.contains(coord));
    }

    #[test]
    fn remove() {
        let mut queue = RebuildQueue::new();
        let coord = ChunkCoord::new(1, 1, 1);

        queue.enqueue(coord, 1.0, 0);
        assert!(queue.contains(coord));

        assert!(queue.remove(coord));
        assert!(!queue.contains(coord));
        assert_eq!(queue.len(), 0);

        // Remove non-existent
        assert!(!queue.remove(coord));
    }

    #[test]
    fn clear() {
        let mut queue = RebuildQueue::new();

        queue.enqueue(ChunkCoord::new(1, 1, 1), 1.0, 0);
        queue.enqueue(ChunkCoord::new(2, 2, 2), 2.0, 0);

        queue.clear();

        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn priority_calculation() {
        // Closer chunks should have higher priority
        let camera = [0.0, 0.0, 0.0];

        let near = [10.0, 0.0, 0.0];
        let far = [100.0, 0.0, 0.0];

        let near_priority = calculate_priority(near, camera);
        let far_priority = calculate_priority(far, camera);

        assert!(near_priority > far_priority);
    }

    #[test]
    fn priority_at_camera_position() {
        let camera = [50.0, 50.0, 50.0];
        let at_camera = camera;

        let priority = calculate_priority(at_camera, camera);

        // Should be very high (1.0 / 0.001 = 1000.0)
        assert!(priority > 100.0);
    }

    #[test]
    fn peek_does_not_remove() {
        let mut queue = RebuildQueue::new();
        queue.enqueue(ChunkCoord::new(1, 1, 1), 1.0, 0);

        let peeked = queue.peek();
        assert!(peeked.is_some());
        assert_eq!(queue.len(), 1);

        let peeked_again = queue.peek();
        assert!(peeked_again.is_some());
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn data_version_preserved() {
        let mut queue = RebuildQueue::new();
        queue.enqueue(ChunkCoord::new(1, 1, 1), 1.0, 42);

        let req = queue.pop().unwrap();
        assert_eq!(req.data_version, 42);
    }
}
