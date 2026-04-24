//! v3 probe slot allocator — chunk-keyed sparse mapping into the cascade
//! payload SSBO. Mirrors `pool::SlotAllocator` (which manages chunk pool
//! slots) so the v3 allocator behaves identically to the existing pattern.
//!
//! The allocator is platform-independent (no GPU dependencies) and runs on
//! the host CPU for unit testing, the same as `pool::SlotAllocator`.

use std::collections::HashMap;

use crate::gi::v3::constants::V3_MAX_PROBE_SLOTS;
use crate::pool::ChunkCoord;

/// Errors returned by `ProbeSlotAllocator::alloc`.
#[derive(Debug, PartialEq, Eq)]
pub enum ProbeAllocError {
    /// All `V3_MAX_PROBE_SLOTS` slots are in use.
    PoolFull,
    /// The same chunk coordinate is already allocated to a slot.
    CoordAlreadyResident,
}

/// Errors returned by `ProbeSlotAllocator::dealloc`.
#[derive(Debug, PartialEq, Eq)]
pub enum ProbeDeallocError {
    SlotNotAllocated,
    SlotOutOfRange,
}

/// CPU-side probe slot directory. Independent from the chunk pool's
/// `SlotAllocator` — a chunk has *both* a chunk slot index (in `pool`) and
/// a v3 probe slot index (here), and they may differ because the v3 budget
/// is smaller than the chunk pool's `MAX_SLOTS`.
pub struct ProbeSlotAllocator {
    /// Free probe slot indices, available for allocation.
    free_slots: Vec<u32>,
    /// Map from chunk coordinate to allocated probe slot index.
    coord_to_slot: HashMap<ChunkCoord, u32>,
    /// Inverse map: slot index → chunk coordinate. None if the slot is free.
    slot_to_coord: Vec<Option<ChunkCoord>>,
}

impl ProbeSlotAllocator {
    /// Create a new allocator with all `V3_MAX_PROBE_SLOTS` slots free.
    pub fn new() -> Self {
        let mut free_slots: Vec<u32> = (0..V3_MAX_PROBE_SLOTS).collect();
        // Reverse so pop() yields 0, 1, 2, ... in order.
        free_slots.reverse();
        Self {
            free_slots,
            coord_to_slot: HashMap::new(),
            slot_to_coord: vec![None; V3_MAX_PROBE_SLOTS as usize],
        }
    }

    /// Allocate a probe slot for the given chunk coordinate.
    pub fn alloc(&mut self, coord: ChunkCoord) -> Result<u32, ProbeAllocError> {
        if self.coord_to_slot.contains_key(&coord) {
            return Err(ProbeAllocError::CoordAlreadyResident);
        }
        let slot = self.free_slots.pop().ok_or(ProbeAllocError::PoolFull)?;
        self.coord_to_slot.insert(coord, slot);
        self.slot_to_coord[slot as usize] = Some(coord);
        Ok(slot)
    }

    /// Deallocate a probe slot, returning the coordinate it held.
    pub fn dealloc(&mut self, slot: u32) -> Result<ChunkCoord, ProbeDeallocError> {
        if slot >= V3_MAX_PROBE_SLOTS {
            return Err(ProbeDeallocError::SlotOutOfRange);
        }
        let coord = self.slot_to_coord[slot as usize]
            .take()
            .ok_or(ProbeDeallocError::SlotNotAllocated)?;
        self.coord_to_slot.remove(&coord);
        self.free_slots.push(slot);
        Ok(coord)
    }

    /// Look up the probe slot for a chunk coordinate. Returns None if no
    /// slot is allocated for this chunk.
    pub fn lookup(&self, coord: &ChunkCoord) -> Option<u32> {
        self.coord_to_slot.get(coord).copied()
    }

    /// Number of currently allocated probe slots.
    pub fn allocated_count(&self) -> u32 {
        V3_MAX_PROBE_SLOTS - self.free_slots.len() as u32
    }

    /// Whether the probe slot pool is full.
    pub fn is_full(&self) -> bool {
        self.free_slots.is_empty()
    }

    /// Reset the allocator to its initial empty state.
    pub fn clear(&mut self) {
        self.coord_to_slot.clear();
        self.slot_to_coord.fill(None);
        self.free_slots.clear();
        self.free_slots.extend((0..V3_MAX_PROBE_SLOTS).rev());
    }

    /// Iterator over all currently allocated `(probe_slot, chunk_coord)` pairs.
    pub fn iter_allocated(&self) -> impl Iterator<Item = (u32, ChunkCoord)> + '_ {
        self.slot_to_coord
            .iter()
            .enumerate()
            .filter_map(|(slot, coord)| coord.map(|c| (slot as u32, c)))
    }
}

impl Default for ProbeSlotAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coord(x: i32, y: i32, z: i32) -> ChunkCoord {
        ChunkCoord { x, y, z }
    }

    #[test]
    fn alloc_returns_distinct_slots() {
        let mut a = ProbeSlotAllocator::new();
        let s0 = a.alloc(coord(0, 0, 0)).unwrap();
        let s1 = a.alloc(coord(1, 0, 0)).unwrap();
        let s2 = a.alloc(coord(0, 1, 0)).unwrap();
        assert_ne!(s0, s1);
        assert_ne!(s0, s2);
        assert_ne!(s1, s2);
        assert_eq!(a.allocated_count(), 3);
    }

    #[test]
    fn alloc_same_coord_twice_errors() {
        let mut a = ProbeSlotAllocator::new();
        a.alloc(coord(0, 0, 0)).unwrap();
        assert_eq!(
            a.alloc(coord(0, 0, 0)),
            Err(ProbeAllocError::CoordAlreadyResident),
        );
    }

    #[test]
    fn dealloc_frees_slot_for_reuse() {
        let mut a = ProbeSlotAllocator::new();
        let s0 = a.alloc(coord(0, 0, 0)).unwrap();
        let returned = a.dealloc(s0).unwrap();
        assert_eq!(returned, coord(0, 0, 0));
        assert_eq!(a.allocated_count(), 0);
        // The same coord can be allocated again.
        let s1 = a.alloc(coord(0, 0, 0)).unwrap();
        assert_eq!(s0, s1, "freed slot should be reused");
    }

    #[test]
    fn dealloc_unallocated_errors() {
        let mut a = ProbeSlotAllocator::new();
        assert_eq!(a.dealloc(0), Err(ProbeDeallocError::SlotNotAllocated));
    }

    #[test]
    fn dealloc_out_of_range_errors() {
        let mut a = ProbeSlotAllocator::new();
        assert_eq!(
            a.dealloc(V3_MAX_PROBE_SLOTS),
            Err(ProbeDeallocError::SlotOutOfRange),
        );
    }

    #[test]
    fn alloc_full_returns_error() {
        let mut a = ProbeSlotAllocator::new();
        for i in 0..V3_MAX_PROBE_SLOTS as i32 {
            a.alloc(coord(i, 0, 0)).unwrap();
        }
        assert!(a.is_full());
        assert_eq!(
            a.alloc(coord(9999, 0, 0)),
            Err(ProbeAllocError::PoolFull),
        );
    }

    #[test]
    fn lookup_finds_allocated_and_misses_free() {
        let mut a = ProbeSlotAllocator::new();
        let s = a.alloc(coord(5, 6, 7)).unwrap();
        assert_eq!(a.lookup(&coord(5, 6, 7)), Some(s));
        assert_eq!(a.lookup(&coord(0, 0, 0)), None);
    }

    #[test]
    fn iter_allocated_yields_all_pairs() {
        let mut a = ProbeSlotAllocator::new();
        let s0 = a.alloc(coord(0, 0, 0)).unwrap();
        let s1 = a.alloc(coord(1, 0, 0)).unwrap();
        let mut pairs: Vec<(u32, ChunkCoord)> = a.iter_allocated().collect();
        pairs.sort_by_key(|p| p.0);
        assert_eq!(pairs, vec![(s0, coord(0, 0, 0)), (s1, coord(1, 0, 0))]);
    }

    #[test]
    fn clear_returns_to_empty_state() {
        let mut a = ProbeSlotAllocator::new();
        a.alloc(coord(0, 0, 0)).unwrap();
        a.alloc(coord(1, 0, 0)).unwrap();
        a.clear();
        assert_eq!(a.allocated_count(), 0);
        assert_eq!(a.lookup(&coord(0, 0, 0)), None);
        // Re-allocation works after clear.
        a.alloc(coord(0, 0, 0)).unwrap();
    }
}
