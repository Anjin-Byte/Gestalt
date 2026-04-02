//! GPU Chunk Pool — constants, slot allocation, and buffer management.
//!
//! Implements the pool design from:
//!   docs/Resident Representation/gpu-chunk-pool.md
//!   docs/Resident Representation/data/*.md
//!
//! Fixed-size slot allocation. Each slot holds:
//! - occupancy atlas: 32 KB (8192 × u32, 4096 columns × 2 words/column)
//! - palette: 512 B (256 × u16, packed 2 per u32)
//! - coord: 16 B (vec4i)
//! - version: 4 B (u32)
//! - flags: 4 B (u32)
//! - AABB: 32 B (2 × vec4f)
//! - occupancy summary: 64 B (16 × u32)
//! - draw metadata: 32 B

use std::collections::HashMap;

// ─── Chunk geometry constants ──────────────────────────────────────────────

/// Padded chunk dimension (storage). Includes 1-voxel padding on all sides.
pub const CS_P: u32 = 64;
/// Usable interior chunk dimension (62 = 64 - 2 padding).
pub const CS: u32 = 62;
/// Voxels per XZ slice (CS_P × CS_P).
pub const CS_P2: u32 = CS_P * CS_P;
/// Total voxels per chunk (CS_P³).
pub const CS_P3: u32 = CS_P * CS_P * CS_P;
/// Number of Y-columns per chunk (one per (x, z) position).
pub const COLUMNS_PER_CHUNK: u32 = CS_P * CS_P;

/// Bricklet dimension (voxels per axis within a bricklet).
pub const BRICKLET_DIM: u32 = 8;
/// Bricklets per chunk axis.
pub const BRICKLETS_PER_AXIS: u32 = CS_P / BRICKLET_DIM;
/// Total bricklets in the XZ plane per chunk.
pub const BRICKLETS_PER_CHUNK: u32 = BRICKLETS_PER_AXIS * BRICKLETS_PER_AXIS;

// ─── Pool sizing constants ─────────────────────────────────────────────────

/// Maximum resident chunk slots.
/// 4096 slots × 289 KB/slot = ~1.2 GB per-slot + 88 MB shared = ~1.3 GB total.
/// Variable index_buf allocation (future) reduces this to ~219 MB.
pub const MAX_SLOTS: u32 = 4096;

/// u32 words per slot in the occupancy atlas (4096 columns × 2 words per u64 column).
pub const OCCUPANCY_WORDS_PER_SLOT: u32 = COLUMNS_PER_CHUNK * 2;
/// Bytes per slot in the occupancy atlas.
pub const OCCUPANCY_BYTES_PER_SLOT: u32 = OCCUPANCY_WORDS_PER_SLOT * 4;

/// Max palette entries per chunk. Capped at 16 (bpe=4) to keep index_buf under 512 MB
/// at 4096 slots. Full 256-entry support requires variable index_buf allocation.
pub const MAX_PALETTE_ENTRIES: u32 = 16;
pub const PALETTE_WORDS_PER_SLOT: u32 = MAX_PALETTE_ENTRIES / 2;
/// Bytes per slot for palette.
pub const PALETTE_BYTES_PER_SLOT: u32 = PALETTE_WORDS_PER_SLOT * 4;

/// Per-voxel palette index buffer — allocated at bpe=4 (supports up to 16 palette entries).
/// 262144 voxels × 4 bits / 32 bits per word = 32768 words.
/// Chunks with >16 materials use bpe=4 and clamp palette to 16 entries.
/// Full bpe=8 support (256 entries) requires variable index_buf allocation (future).
pub const INDEX_BUF_MAX_BPE: u32 = 4;
pub const INDEX_BUF_WORDS_PER_SLOT: u32 = CS_P3 * INDEX_BUF_MAX_BPE / 32;
/// Bytes per slot for index buffer.
pub const INDEX_BUF_BYTES_PER_SLOT: u32 = INDEX_BUF_WORDS_PER_SLOT * 4;

/// Palette metadata: 1 u32 per slot.
/// Bits 0–15: palette_size (u16). Bits 16–23: bits_per_entry (u8). Bits 24–31: reserved.
pub const PALETTE_META_BYTES: u32 = 4;

/// Chunk coordinate: vec4i = 16 bytes.
pub const COORD_BYTES: u32 = 16;

/// Chunk version: u32 = 4 bytes.
pub const VERSION_BYTES: u32 = 4;

/// Chunk flags: u32 = 4 bytes.
pub const FLAGS_BYTES: u32 = 4;

/// Chunk AABB: 2 × vec4f = 32 bytes.
pub const AABB_BYTES: u32 = 32;

/// Occupancy summary: 16 × u32 = 64 bytes (512 bits for 8×8×8 bricklets).
pub const SUMMARY_WORDS_PER_SLOT: u32 = 16;
pub const SUMMARY_BYTES_PER_SLOT: u32 = SUMMARY_WORDS_PER_SLOT * 4;

/// Draw metadata per slot: 32 bytes.
pub const DRAW_META_BYTES: u32 = 32;

/// Bytes per vertex (vec3f position + u32 packed normal/material = 16 bytes).
pub const VERTEX_BYTES: u32 = 16;
/// Bytes per index (u32).
pub const INDEX_BYTES: u32 = 4;

// ─── Mesh pool budget (variable allocation) ───────────────────────────────
// See: docs/Resident Representation/variable-mesh-pool.md

/// Total vertex pool capacity (shared across all slots).
pub const MESH_VERTEX_POOL_CAPACITY: u32 = 4_194_304; // 4M vertices
/// Total index pool capacity (shared across all slots).
pub const MESH_INDEX_POOL_CAPACITY: u32 = 6_291_456; // 6M indices

/// Mesh offset table entry: 20 bytes per slot (5 × u32: vertex_offset, vertex_count, index_offset, index_count, write_counter).
pub const MESH_OFFSET_ENTRY_BYTES: u32 = 20;
/// Mesh counts buffer: 4 bytes per slot (u32 quad count, written by count pass).
pub const MESH_COUNTS_ENTRY_BYTES: u32 = 4;

// ─── Wireframe (still fixed allocation, to be variablized in Phase 5) ─────

/// Legacy per-slot limits — used ONLY by wireframe. Deprecated for mesh pool.
pub const MAX_VERTS_PER_CHUNK: u32 = 16_384;
pub const MAX_INDICES_PER_CHUNK: u32 = 24_576;
/// Maximum wireframe edge indices per chunk (4 edges × 2 indices per quad).
pub const MAX_WIRE_INDICES_PER_CHUNK: u32 = MAX_INDICES_PER_CHUNK / 6 * 8;
/// Total wireframe index buffer size.
pub const TOTAL_WIRE_INDEX_BYTES: u64 = MAX_WIRE_INDICES_PER_CHUNK as u64 * INDEX_BYTES as u64 * MAX_SLOTS as u64;

/// Maximum indirect draw calls per frame (= MAX_SLOTS for chunk-level draws).
pub const MAX_DRAWS: u32 = MAX_SLOTS;

// ─── Material constants ────────────────────────────────────────────────────

/// Reserved MaterialId for empty/air voxels. Never rendered.
pub const MATERIAL_EMPTY: u16 = 0;
/// Reserved MaterialId for default/fallback material.
pub const MATERIAL_DEFAULT: u16 = 1;
/// Maximum materials in the global material table.
pub const MAX_MATERIALS: u32 = 4096;
/// Bytes per MaterialEntry (4 × u32 packed f16 pairs).
pub const MATERIAL_ENTRY_BYTES: u32 = 16;

// ─── Derived totals ────────────────────────────────────────────────────────

/// Total occupancy atlas buffer size for all slots.
pub const TOTAL_OCCUPANCY_BYTES: u64 = OCCUPANCY_BYTES_PER_SLOT as u64 * MAX_SLOTS as u64;
/// Total palette buffer size for all slots.
pub const TOTAL_PALETTE_BYTES: u64 = PALETTE_BYTES_PER_SLOT as u64 * MAX_SLOTS as u64;
/// Total index buffer pool size for all slots.
pub const TOTAL_INDEX_BUF_BYTES: u64 = INDEX_BUF_BYTES_PER_SLOT as u64 * MAX_SLOTS as u64;
/// Total palette metadata buffer size for all slots.
pub const TOTAL_PALETTE_META_BYTES: u64 = PALETTE_META_BYTES as u64 * MAX_SLOTS as u64;
/// Total vertex pool size (budget-based, not per-slot × MAX).
pub const TOTAL_VERTEX_BYTES: u64 = MESH_VERTEX_POOL_CAPACITY as u64 * VERTEX_BYTES as u64;
/// Total index pool size (budget-based).
pub const TOTAL_INDEX_BYTES: u64 = MESH_INDEX_POOL_CAPACITY as u64 * INDEX_BYTES as u64;
/// Total material table size.
pub const TOTAL_MATERIAL_BYTES: u64 = MAX_MATERIALS as u64 * MATERIAL_ENTRY_BYTES as u64;

// ─── Draw metadata struct ─────────────────────────────────────────────────

/// Per-slot draw metadata written by R-1, consumed by R-4 and render passes.
/// Matches the GPU layout exactly (32 bytes = 8 × u32).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DrawMeta {
    pub vertex_offset: u32,
    pub vertex_count: u32,
    pub index_offset: u32,
    pub index_count: u32,
    pub material_base: u32,
    pub _pad: [u32; 3],
}

const _: () = assert!(
    std::mem::size_of::<DrawMeta>() == DRAW_META_BYTES as usize,
    "DrawMeta must be exactly 32 bytes"
);

// ─── Face direction constants ─────────────────────────────────────────────

pub const FACE_POS_Y: usize = 0;
pub const FACE_NEG_Y: usize = 1;
pub const FACE_POS_X: usize = 2;
pub const FACE_NEG_X: usize = 3;
pub const FACE_POS_Z: usize = 4;
pub const FACE_NEG_Z: usize = 5;
pub const NUM_FACES: usize = 6;

// ─── Static assertions ────────────────────────────────────────────────────
// These run at compile time. If any fails, the build breaks with a clear message.

const _: () = assert!(CS_P == 64, "Chunk padded dimension must be 64");
const _: () = assert!(CS == 62, "Chunk usable dimension must be 62 (64 - 2 padding)");
const _: () = assert!(CS_P3 == 262_144, "Total voxels per chunk must be 64^3 = 262144");
const _: () = assert!(COLUMNS_PER_CHUNK == 4096, "Columns per chunk must be 64*64 = 4096");
const _: () = assert!(OCCUPANCY_WORDS_PER_SLOT == 2 * COLUMNS_PER_CHUNK, "Occupancy must be 2 u32 per column (= 1 u64)");
const _: () = assert!(OCCUPANCY_BYTES_PER_SLOT == 32768, "Occupancy must be 32 KB per slot (4096 columns × 8 bytes/column)");
const _: () = assert!(BRICKLET_DIM == 8, "Bricklet dimension must be 8");
const _: () = assert!(BRICKLETS_PER_AXIS == 8, "Must have 8 bricklets per axis (64/8)");
const _: () = assert!(BRICKLETS_PER_CHUNK == 64, "Must have 64 bricklets per chunk XZ plane");
const _: () = assert!(SUMMARY_WORDS_PER_SLOT * 32 >= BRICKLETS_PER_CHUNK * BRICKLETS_PER_AXIS,
    "Summary must have enough bits for all bricklets (8^3 = 512)");
const _: () = assert!(MAX_PALETTE_ENTRIES <= 256, "Palette limited to 256 entries (8-bit index max)");
const _: () = assert!(PALETTE_BYTES_PER_SLOT == 32, "Fixed palette allocation must be 32 bytes (16 entries × 2 bytes, packed 2 per u32 = 8 words × 4)");
const _: () = assert!(MATERIAL_ENTRY_BYTES == 16, "MaterialEntry must be 16 bytes (4 × u32 packed f16)");
const _: () = assert!(TOTAL_MATERIAL_BYTES == 65536, "Material table must be 64 KB");
const _: () = assert!(MAX_VERTS_PER_CHUNK % 4 == 0, "MAX_VERTS must be multiple of 4 (quad vertices)");
const _: () = assert!(MAX_INDICES_PER_CHUNK % 6 == 0, "MAX_INDICES must be multiple of 6 (quad indices: 2 tris × 3)");
const _: () = assert!(MAX_DRAWS == MAX_SLOTS, "MAX_DRAWS must equal MAX_SLOTS for chunk-level draws");

// F1: WGSL shaders hardcode MAX_SLOTS as a compile-time constant for bounds guards.
// If this value changes, update build_indirect.wgsl and build_wireframe.wgsl.
const _: () = assert!(MAX_SLOTS == 4096, "WGSL shaders hardcode MAX_SLOTS=4096 — update shaders if this changes");

// F5: DrawIndexedIndirect.base_vertex is i32 in the WebGPU spec.
// We write slot * MAX_VERTS_PER_CHUNK as u32. This must fit in i32 (< 2^31).
const _: () = assert!(
    (MAX_SLOTS as u64) * (MAX_VERTS_PER_CHUNK as u64) <= 2_147_483_647,
    "base_vertex (slot * MAX_VERTS) must fit in i32 for DrawIndexedIndirect"
);

// F5: Same check for wireframe base_vertex.
const _: () = assert!(
    (MAX_SLOTS as u64) * (MAX_VERTS_PER_CHUNK as u64) <= 2_147_483_647,
    "wireframe base_vertex must fit in i32"
);

// ─── Indirect draw constants ──────────────────────────────────────────────

/// Bytes per DrawIndexedIndirect struct (5 × u32).
pub const DRAW_INDIRECT_BYTES: u32 = 20;
/// Total indirect draw buffer size.
pub const TOTAL_INDIRECT_BYTES: u64 = DRAW_INDIRECT_BYTES as u64 * MAX_DRAWS as u64;

// ─── Slot allocator ──────────────────────────────────────────────────────

/// World-space chunk coordinate (signed, in chunk units).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkCoord {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// Error returned when a slot cannot be allocated.
#[derive(Debug, PartialEq, Eq)]
pub enum AllocError {
    PoolFull,
    CoordAlreadyResident,
}

/// Error returned when a slot cannot be deallocated.
#[derive(Debug, PartialEq, Eq)]
pub enum DeallocError {
    SlotNotAllocated,
    SlotOutOfRange,
}

/// CPU-side slot directory. Manages the freelist and coord↔slot maps.
/// Platform-independent — no GPU dependencies, fully testable natively.
pub struct SlotAllocator {
    /// Free slot indices, available for allocation.
    free_slots: Vec<u32>,
    /// Map from chunk coordinate to allocated slot index.
    coord_to_slot: HashMap<ChunkCoord, u32>,
    /// Inverse map: slot index → chunk coordinate. None if the slot is free.
    slot_to_coord: Vec<Option<ChunkCoord>>,
}

impl SlotAllocator {
    /// Create a new allocator with all MAX_SLOTS slots free.
    pub fn new() -> Self {
        let mut free_slots: Vec<u32> = (0..MAX_SLOTS).collect();
        // Reverse so pop() yields 0, 1, 2, ... in order.
        free_slots.reverse();
        Self {
            free_slots,
            coord_to_slot: HashMap::new(),
            slot_to_coord: vec![None; MAX_SLOTS as usize],
        }
    }

    /// Allocate a slot for the given chunk coordinate.
    /// Returns the slot index on success.
    pub fn alloc(&mut self, coord: ChunkCoord) -> Result<u32, AllocError> {
        if self.coord_to_slot.contains_key(&coord) {
            return Err(AllocError::CoordAlreadyResident);
        }
        let slot = self.free_slots.pop().ok_or(AllocError::PoolFull)?;
        self.coord_to_slot.insert(coord, slot);
        self.slot_to_coord[slot as usize] = Some(coord);
        Ok(slot)
    }

    /// Deallocate a slot, returning the coordinate that was stored there.
    pub fn dealloc(&mut self, slot: u32) -> Result<ChunkCoord, DeallocError> {
        if slot >= MAX_SLOTS {
            return Err(DeallocError::SlotOutOfRange);
        }
        let coord = self.slot_to_coord[slot as usize]
            .take()
            .ok_or(DeallocError::SlotNotAllocated)?;
        self.coord_to_slot.remove(&coord);
        self.free_slots.push(slot);
        Ok(coord)
    }

    /// Look up the slot for a chunk coordinate. Returns None if not resident.
    pub fn lookup(&self, coord: &ChunkCoord) -> Option<u32> {
        self.coord_to_slot.get(coord).copied()
    }

    /// Look up the coordinate stored in a slot. Returns None if the slot is free.
    pub fn coord_of(&self, slot: u32) -> Option<ChunkCoord> {
        self.slot_to_coord.get(slot as usize).copied().flatten()
    }

    /// Number of free slots remaining.
    pub fn free_count(&self) -> u32 {
        self.free_slots.len() as u32
    }

    /// Number of currently allocated (resident) slots.
    pub fn resident_count(&self) -> u32 {
        MAX_SLOTS - self.free_count()
    }

    /// Whether the pool is completely full.
    pub fn is_full(&self) -> bool {
        self.free_slots.is_empty()
    }

    /// Deallocate all slots, returning the pool to its initial empty state.
    pub fn clear(&mut self) {
        self.coord_to_slot.clear();
        self.slot_to_coord.fill(None);
        self.free_slots.clear();
        self.free_slots.extend((0..MAX_SLOTS).rev());
    }

    /// Iterator over all currently allocated (slot, coord) pairs.
    pub fn allocated_slots(&self) -> impl Iterator<Item = (u32, ChunkCoord)> + '_ {
        self.slot_to_coord.iter().enumerate().filter_map(|(slot, coord)| {
            coord.map(|c| (slot as u32, c))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occupancy_addressing_roundtrip() {
        // Verify column_index formula covers all (x, z) without collision
        let mut seen = vec![false; COLUMNS_PER_CHUNK as usize];
        for x in 0..CS_P {
            for z in 0..CS_P {
                let col = (x * CS_P + z) as usize;
                assert!(!seen[col], "Duplicate column index at ({x}, {z}) = {col}");
                seen[col] = true;
            }
        }
        assert!(seen.iter().all(|&s| s), "Not all column indices covered");
    }

    #[test]
    fn occupancy_y_bit_addressing() {
        // Verify Y-bit addressing within a column
        for y in 0..CS_P {
            let u32_index = (y >> 5) as usize;
            let bit_within = y & 31;
            assert!(u32_index < 2, "u32_index out of range for y={y}");
            assert!(bit_within < 32, "bit_within out of range for y={y}");
            // Verify bit is uniquely addressable
            let mask = 1u32 << bit_within;
            assert_ne!(mask, 0);
        }
    }

    #[test]
    fn bricklet_addressing_covers_all() {
        // Verify bricklet (bx, by, bz) covers all 512 bricklets
        let mut seen = vec![false; 512];
        for bx in 0..BRICKLETS_PER_AXIS {
            for by in 0..BRICKLETS_PER_AXIS {
                for bz in 0..BRICKLETS_PER_AXIS {
                    let idx = (bx * 64 + by * 8 + bz) as usize;
                    assert!(idx < 512, "Bricklet index out of range: ({bx},{by},{bz}) = {idx}");
                    assert!(!seen[idx], "Duplicate bricklet index at ({bx},{by},{bz})");
                    seen[idx] = true;
                }
            }
        }
        assert!(seen.iter().all(|&s| s), "Not all bricklet indices covered");
    }

    #[test]
    fn bricklet_maps_to_correct_voxel_range() {
        // Verify bricklet (bx, by, bz) maps to the correct 8^3 voxel region
        for bx in 0..BRICKLETS_PER_AXIS {
            for bz in 0..BRICKLETS_PER_AXIS {
                let x_start = bx * BRICKLET_DIM;
                let z_start = bz * BRICKLET_DIM;
                assert!(x_start + BRICKLET_DIM <= CS_P);
                assert!(z_start + BRICKLET_DIM <= CS_P);
            }
        }
    }

    #[test]
    fn palette_bits_per_entry_valid() {
        // Verify the palette_size → bits_per_entry mapping covers all cases
        let valid_bpe = [1u32, 2, 4, 8];
        for palette_size in 1..=MAX_PALETTE_ENTRIES {
            let bpe = match palette_size {
                1..=2 => 1,
                3..=4 => 2,
                5..=16 => 4,
                17..=256 => 8,
                _ => unreachable!(),
            };
            assert!(valid_bpe.contains(&bpe), "Invalid bpe {bpe} for palette_size {palette_size}");
            // Verify bpe is sufficient to index the palette
            assert!((1u32 << bpe) >= palette_size, "bpe {bpe} insufficient for palette_size {palette_size}");
        }
    }

    #[test]
    fn index_buffer_size_at_each_bpe() {
        // Verify index buffer word count at each valid bits_per_entry
        for bpe in [1u32, 2, 4, 8] {
            let total_bits = CS_P3 as u64 * bpe as u64;
            let num_words = (total_bits + 31) / 32;
            // Must fit in a reasonable buffer
            let num_bytes = num_words * 4;
            assert!(num_bytes <= 256 * 1024, "Index buffer at bpe={bpe} exceeds 256 KB: {num_bytes} bytes");
        }
    }

    #[test]
    fn memory_budget_at_max_slots() {
        // Verify total GPU memory doesn't exceed reasonable bounds
        let total = TOTAL_OCCUPANCY_BYTES
            + TOTAL_PALETTE_BYTES
            + TOTAL_VERTEX_BYTES
            + TOTAL_INDEX_BYTES
            + TOTAL_MATERIAL_BYTES
            + (COORD_BYTES as u64 * MAX_SLOTS as u64)
            + (VERSION_BYTES as u64 * MAX_SLOTS as u64)
            + (FLAGS_BYTES as u64 * MAX_SLOTS as u64)
            + (AABB_BYTES as u64 * MAX_SLOTS as u64)
            + (SUMMARY_BYTES_PER_SLOT as u64 * MAX_SLOTS as u64)
            + (DRAW_META_BYTES as u64 * MAX_SLOTS as u64);

        let total_mb = total / (1024 * 1024);
        // At 1024 slots: should be well under 2 GB
        assert!(total_mb < 2048, "Total GPU budget {total_mb} MB exceeds 2 GB");
        // Print for documentation
        eprintln!("Total GPU memory at {MAX_SLOTS} slots: {total_mb} MB");
        eprintln!("  Occupancy atlas: {} MB", TOTAL_OCCUPANCY_BYTES / (1024 * 1024));
        eprintln!("  Palette:         {} MB", TOTAL_PALETTE_BYTES / (1024 * 1024));
        eprintln!("  Vertex pool:     {} MB", TOTAL_VERTEX_BYTES / (1024 * 1024));
        eprintln!("  Index pool:      {} MB", TOTAL_INDEX_BYTES / (1024 * 1024));
        eprintln!("  Material table:  {} KB", TOTAL_MATERIAL_BYTES / 1024);
    }

    #[test]
    fn material_id_reserved_range() {
        assert_eq!(MATERIAL_EMPTY, 0);
        assert_eq!(MATERIAL_DEFAULT, 1);
        assert!(MATERIAL_DEFAULT < MAX_MATERIALS as u16);
    }

    #[test]
    fn draw_indirect_struct_size() {
        // WebGPU DrawIndexedIndirect: 5 × u32 = 20 bytes
        let draw_indirect_bytes = 20u32;
        let total_indirect = draw_indirect_bytes as u64 * MAX_DRAWS as u64;
        assert!(total_indirect < 1024 * 1024, "Indirect draw buffer exceeds 1 MB");
    }

    // ─── SlotAllocator tests ──────────────────────────────────────────────

    #[test]
    fn alloc_returns_sequential_slots() {
        let mut alloc = SlotAllocator::new();
        let s0 = alloc.alloc(ChunkCoord { x: 0, y: 0, z: 0 }).unwrap();
        let s1 = alloc.alloc(ChunkCoord { x: 1, y: 0, z: 0 }).unwrap();
        let s2 = alloc.alloc(ChunkCoord { x: 2, y: 0, z: 0 }).unwrap();
        assert_eq!(s0, 0);
        assert_eq!(s1, 1);
        assert_eq!(s2, 2);
    }

    #[test]
    fn alloc_dealloc_roundtrip() {
        let mut alloc = SlotAllocator::new();
        let coord = ChunkCoord { x: 5, y: -3, z: 10 };
        let slot = alloc.alloc(coord).unwrap();
        let returned = alloc.dealloc(slot).unwrap();
        assert_eq!(returned, coord);
        assert!(alloc.lookup(&coord).is_none());
        assert!(alloc.coord_of(slot).is_none());
    }

    #[test]
    fn alloc_duplicate_coord_fails() {
        let mut alloc = SlotAllocator::new();
        let coord = ChunkCoord { x: 1, y: 2, z: 3 };
        alloc.alloc(coord).unwrap();
        assert_eq!(alloc.alloc(coord), Err(AllocError::CoordAlreadyResident));
    }

    #[test]
    fn pool_exhaustion() {
        let mut alloc = SlotAllocator::new();
        for i in 0..MAX_SLOTS {
            alloc.alloc(ChunkCoord { x: i as i32, y: 0, z: 0 }).unwrap();
        }
        assert!(alloc.is_full());
        assert_eq!(alloc.free_count(), 0);
        assert_eq!(
            alloc.alloc(ChunkCoord { x: -1, y: 0, z: 0 }),
            Err(AllocError::PoolFull)
        );
    }

    #[test]
    fn dealloc_enables_realloc() {
        let mut alloc = SlotAllocator::new();
        for i in 0..MAX_SLOTS {
            alloc.alloc(ChunkCoord { x: i as i32, y: 0, z: 0 }).unwrap();
        }
        assert!(alloc.is_full());
        alloc.dealloc(42).unwrap();
        assert!(!alloc.is_full());
        let slot = alloc.alloc(ChunkCoord { x: -1, y: -1, z: -1 }).unwrap();
        assert_eq!(slot, 42);
    }

    #[test]
    fn lookup_after_alloc() {
        let mut alloc = SlotAllocator::new();
        let coord = ChunkCoord { x: 7, y: 8, z: 9 };
        let slot = alloc.alloc(coord).unwrap();
        assert_eq!(alloc.lookup(&coord), Some(slot));
    }

    #[test]
    fn lookup_after_dealloc() {
        let mut alloc = SlotAllocator::new();
        let coord = ChunkCoord { x: 7, y: 8, z: 9 };
        let slot = alloc.alloc(coord).unwrap();
        alloc.dealloc(slot).unwrap();
        assert_eq!(alloc.lookup(&coord), None);
    }

    #[test]
    fn coord_of_roundtrip() {
        let mut alloc = SlotAllocator::new();
        let coord = ChunkCoord { x: -10, y: 20, z: -30 };
        let slot = alloc.alloc(coord).unwrap();
        assert_eq!(alloc.coord_of(slot), Some(coord));
    }

    #[test]
    fn free_count_tracking() {
        let mut alloc = SlotAllocator::new();
        assert_eq!(alloc.free_count(), MAX_SLOTS);
        assert_eq!(alloc.resident_count(), 0);

        alloc.alloc(ChunkCoord { x: 0, y: 0, z: 0 }).unwrap();
        assert_eq!(alloc.free_count(), MAX_SLOTS - 1);
        assert_eq!(alloc.resident_count(), 1);

        alloc.alloc(ChunkCoord { x: 1, y: 0, z: 0 }).unwrap();
        assert_eq!(alloc.free_count(), MAX_SLOTS - 2);
        assert_eq!(alloc.resident_count(), 2);

        alloc.dealloc(0).unwrap();
        assert_eq!(alloc.free_count(), MAX_SLOTS - 1);
        assert_eq!(alloc.resident_count(), 1);
    }

    #[test]
    fn dealloc_invalid_slot() {
        let mut alloc = SlotAllocator::new();
        // Unallocated slot
        assert_eq!(alloc.dealloc(0), Err(DeallocError::SlotNotAllocated));
        // Out of range
        assert_eq!(alloc.dealloc(MAX_SLOTS), Err(DeallocError::SlotOutOfRange));
        assert_eq!(alloc.dealloc(u32::MAX), Err(DeallocError::SlotOutOfRange));
    }

    #[test]
    fn rapid_alloc_dealloc_cycle() {
        let mut alloc = SlotAllocator::new();
        let initial_free = alloc.free_count();
        for i in 0..100u32 {
            let coord = ChunkCoord { x: i as i32, y: 0, z: 0 };
            let slot = alloc.alloc(coord).unwrap();
            alloc.dealloc(slot).unwrap();
        }
        assert_eq!(alloc.free_count(), initial_free, "Leaked slots after alloc/dealloc cycle");
        assert_eq!(alloc.resident_count(), 0);
    }

    #[test]
    fn negative_coords() {
        let mut alloc = SlotAllocator::new();
        let coords = [
            ChunkCoord { x: -1, y: -1, z: -1 },
            ChunkCoord { x: i32::MIN, y: 0, z: 0 },
            ChunkCoord { x: 0, y: i32::MAX, z: 0 },
            ChunkCoord { x: i32::MIN, y: i32::MAX, z: i32::MIN },
        ];
        for coord in &coords {
            let slot = alloc.alloc(*coord).unwrap();
            assert_eq!(alloc.lookup(coord), Some(slot));
            assert_eq!(alloc.coord_of(slot), Some(*coord));
        }
        assert_eq!(alloc.resident_count(), 4);
    }
}
