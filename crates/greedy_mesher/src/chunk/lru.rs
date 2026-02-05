//! LRU access tracking for chunk eviction.
//!
//! Uses a monotonic counter to track access times. Chunks with the
//! oldest access time are evicted first when the memory budget is exceeded.

use std::collections::HashMap;
use super::coord::ChunkCoord;

/// Tracks chunk access times using a monotonic counter.
///
/// Each call to [`touch`](LruTracker::touch) records the current time
/// and increments the counter. Chunks with the smallest access time
/// are the least recently used.
pub struct LruTracker {
    access_times: HashMap<ChunkCoord, u64>,
    current_time: u64,
}

impl LruTracker {
    /// Create a new empty tracker.
    pub fn new() -> Self {
        Self {
            access_times: HashMap::new(),
            current_time: 0,
        }
    }

    /// Record an access to a chunk.
    pub fn touch(&mut self, coord: ChunkCoord) {
        self.current_time += 1;
        self.access_times.insert(coord, self.current_time);
    }

    /// Get the access time for a chunk, or `None` if never accessed.
    pub fn get_access_time(&self, coord: ChunkCoord) -> Option<u64> {
        self.access_times.get(&coord).copied()
    }

    /// Get all tracked chunks sorted by access time (oldest first).
    pub fn get_lru_sorted(&self) -> Vec<(ChunkCoord, u64)> {
        let mut entries: Vec<_> = self.access_times
            .iter()
            .map(|(&coord, &time)| (coord, time))
            .collect();
        entries.sort_by_key(|&(_, time)| time);
        entries
    }

    /// Remove tracking for a chunk.
    pub fn remove(&mut self, coord: ChunkCoord) {
        self.access_times.remove(&coord);
    }

    /// Number of tracked chunks.
    pub fn len(&self) -> usize {
        self.access_times.len()
    }

    /// Whether the tracker is empty.
    pub fn is_empty(&self) -> bool {
        self.access_times.is_empty()
    }

    /// Clear all tracking data.
    pub fn clear(&mut self) {
        self.access_times.clear();
        self.current_time = 0;
    }
}

impl Default for LruTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_tracker_is_empty() {
        let tracker = LruTracker::new();
        assert!(tracker.is_empty());
        assert_eq!(tracker.len(), 0);
    }

    #[test]
    fn touch_records_access() {
        let mut tracker = LruTracker::new();
        let coord = ChunkCoord::new(1, 2, 3);

        tracker.touch(coord);

        assert_eq!(tracker.len(), 1);
        assert!(tracker.get_access_time(coord).is_some());
    }

    #[test]
    fn touch_increments_time() {
        let mut tracker = LruTracker::new();
        let a = ChunkCoord::new(0, 0, 0);
        let b = ChunkCoord::new(1, 0, 0);

        tracker.touch(a);
        tracker.touch(b);

        let time_a = tracker.get_access_time(a).unwrap();
        let time_b = tracker.get_access_time(b).unwrap();
        assert!(time_b > time_a);
    }

    #[test]
    fn multiple_touches_update_time() {
        let mut tracker = LruTracker::new();
        let a = ChunkCoord::new(0, 0, 0);
        let b = ChunkCoord::new(1, 0, 0);

        tracker.touch(a); // time=1
        tracker.touch(b); // time=2
        tracker.touch(a); // time=3, a is now newer than b

        let time_a = tracker.get_access_time(a).unwrap();
        let time_b = tracker.get_access_time(b).unwrap();
        assert!(time_a > time_b);
    }

    #[test]
    fn get_lru_sorted_oldest_first() {
        let mut tracker = LruTracker::new();
        let a = ChunkCoord::new(0, 0, 0);
        let b = ChunkCoord::new(1, 0, 0);
        let c = ChunkCoord::new(2, 0, 0);

        tracker.touch(c);
        tracker.touch(a);
        tracker.touch(b);

        let sorted = tracker.get_lru_sorted();
        assert_eq!(sorted.len(), 3);
        assert_eq!(sorted[0].0, c); // oldest
        assert_eq!(sorted[1].0, a);
        assert_eq!(sorted[2].0, b); // newest
    }

    #[test]
    fn remove_clears_tracking() {
        let mut tracker = LruTracker::new();
        let coord = ChunkCoord::new(1, 2, 3);

        tracker.touch(coord);
        assert_eq!(tracker.len(), 1);

        tracker.remove(coord);
        assert_eq!(tracker.len(), 0);
        assert!(tracker.get_access_time(coord).is_none());
    }

    #[test]
    fn clear_resets_everything() {
        let mut tracker = LruTracker::new();
        tracker.touch(ChunkCoord::new(0, 0, 0));
        tracker.touch(ChunkCoord::new(1, 0, 0));

        tracker.clear();

        assert!(tracker.is_empty());
    }

    #[test]
    fn untracked_chunk_returns_none() {
        let tracker = LruTracker::new();
        assert!(tracker.get_access_time(ChunkCoord::ZERO).is_none());
    }
}
