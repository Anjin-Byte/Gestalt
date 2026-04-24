//! Procedural test scene — occupancy builders and geometry generators.
//!
//! Platform-independent. No GPU dependencies. Generates chunk occupancy data
//! that can be uploaded via `ChunkPool::upload_chunk`.

use crate::pool::*;

// ─── Occupancy builder ──────────────────────────────────────────────────

/// Writable occupancy grid for a single 64³ chunk.
/// Column-major layout: column_index = x * CS_P + z, bit y in that column's u64.
pub struct OccupancyBuilder {
    /// 8192 u32 words (4096 columns × 2 words per column).
    words: Vec<u32>,
}

impl OccupancyBuilder {
    pub fn new() -> Self {
        Self {
            words: vec![0u32; OCCUPANCY_WORDS_PER_SLOT as usize],
        }
    }

    /// Set a single voxel at (x, y, z). All coords in [0, 63].
    #[inline]
    pub fn set(&mut self, x: u32, y: u32, z: u32) {
        debug_assert!(x < CS_P && y < CS_P && z < CS_P, "({x},{y},{z}) out of range");
        let col = (x * CS_P + z) as usize;
        let word_offset = col * 2;
        let u32_idx = word_offset + (y >> 5) as usize;
        let bit = y & 31;
        self.words[u32_idx] |= 1 << bit;
    }

    /// Clear a single voxel at (x, y, z).
    #[inline]
    pub fn clear(&mut self, x: u32, y: u32, z: u32) {
        debug_assert!(x < CS_P && y < CS_P && z < CS_P);
        let col = (x * CS_P + z) as usize;
        let word_offset = col * 2;
        let u32_idx = word_offset + (y >> 5) as usize;
        let bit = y & 31;
        self.words[u32_idx] &= !(1 << bit);
    }

    /// Test whether a voxel is set.
    #[inline]
    pub fn get(&self, x: u32, y: u32, z: u32) -> bool {
        debug_assert!(x < CS_P && y < CS_P && z < CS_P);
        let col = (x * CS_P + z) as usize;
        let word_offset = col * 2;
        let u32_idx = word_offset + (y >> 5) as usize;
        let bit = y & 31;
        (self.words[u32_idx] >> bit) & 1 != 0
    }

    /// Return the occupancy data as a slice for upload.
    pub fn as_words(&self) -> &[u32] {
        &self.words
    }

    /// Count total occupied voxels.
    pub fn popcount(&self) -> u32 {
        self.words.iter().map(|w| w.count_ones()).sum()
    }
}

// ─── Palette builder ────────────────────────────────────────────────────

/// Simple palette: maps voxel positions to material IDs via a per-voxel grid.
/// For the test scene we use a small fixed palette.
pub struct PaletteBuilder {
    entries: Vec<u16>,
}

impl PaletteBuilder {
    pub fn new() -> Self {
        Self {
            entries: vec![MATERIAL_EMPTY],
        }
    }

    /// Add a material to the palette. Returns the palette index.
    /// Silently returns existing index if already present.
    pub fn add(&mut self, material_id: u16) -> u8 {
        if let Some(pos) = self.entries.iter().position(|&m| m == material_id) {
            return pos as u8;
        }
        if self.entries.len() >= MAX_PALETTE_ENTRIES as usize {
            // Palette full (256 entries). Map overflow to entry 0.
            return 0;
        }
        self.entries.push(material_id);
        (self.entries.len() - 1) as u8
    }

    /// Return palette data packed as u32 words (2 × u16 per word).
    pub fn as_words(&self) -> Vec<u32> {
        let mut words = vec![0u32; (self.entries.len() + 1) / 2];
        for (i, &mat_id) in self.entries.iter().enumerate() {
            let word_idx = i / 2;
            let shift = (i & 1) * 16;
            words[word_idx] |= (mat_id as u32) << shift;
        }
        words
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

// ─── Per-voxel palette index buffer ────────────────────────────────────

/// Per-voxel palette indices, bitpacked at variable bit width.
///
/// Maps every voxel in a 64³ chunk to its palette entry index. The raw
/// storage is u8-per-voxel (262144 entries). The `pack` method bitpacks
/// at the specified `bits_per_entry` for GPU upload.
///
/// See: docs/Resident Representation/data/chunk-index-buf.md
pub struct IndexBufBuilder {
    /// Raw per-voxel palette index. Layout: index_map[x * CS_P² + y * CS_P + z].
    /// 0 for unoccupied voxels (palette entry 0 = MATERIAL_EMPTY).
    index_map: Vec<u8>,
}

impl IndexBufBuilder {
    pub fn new() -> Self {
        Self {
            index_map: vec![0u8; CS_P3 as usize],
        }
    }

    /// Set the palette index for voxel at (x, y, z). All coords in [0, 63].
    #[inline]
    pub fn set(&mut self, x: u32, y: u32, z: u32, palette_idx: u8) {
        debug_assert!(x < CS_P && y < CS_P && z < CS_P, "({x},{y},{z}) out of range");
        let idx = (x * CS_P * CS_P + y * CS_P + z) as usize;
        self.index_map[idx] = palette_idx;
    }

    /// Get the palette index for voxel at (x, y, z).
    #[inline]
    pub fn get(&self, x: u32, y: u32, z: u32) -> u8 {
        debug_assert!(x < CS_P && y < CS_P && z < CS_P);
        self.index_map[(x * CS_P * CS_P + y * CS_P + z) as usize]
    }

    /// Compute the minimum bits_per_entry for a given palette size.
    /// Returns 1, 2, 4, or 8 — always a power of two that divides 32,
    /// so bitpacked entries never span u32 word boundaries (IDX-1).
    pub fn bits_per_entry(palette_size: usize) -> u8 {
        match palette_size {
            0..=2 => 1,
            3..=4 => 2,
            5..=16 => 4,
            _ => 8,
        }
    }

    /// Pack per-voxel indices into u32 words at the given bit width.
    ///
    /// `bpe` must be 1, 2, 4, or 8. Returns `ceil(262144 * bpe / 32)` words.
    pub fn pack(&self, bpe: u8) -> Vec<u32> {
        debug_assert!(matches!(bpe, 1 | 2 | 4 | 8), "bpe must be 1, 2, 4, or 8");
        let total_bits = CS_P3 as usize * bpe as usize;
        let word_count = (total_bits + 31) / 32;
        let mut words = vec![0u32; word_count];
        let mask = (1u32 << bpe) - 1;

        for (i, &val) in self.index_map.iter().enumerate() {
            let bit_offset = i * bpe as usize;
            let word_idx = bit_offset >> 5;
            let bit_within = bit_offset & 31;
            words[word_idx] |= ((val as u32) & mask) << bit_within;
        }

        words
    }

    /// Build the packed palette_meta u32 for a given palette size.
    ///
    /// Layout: bits 0–15 = palette_size, bits 16–23 = bits_per_entry, bits 24–31 = 0.
    pub fn palette_meta(palette_size: usize) -> u32 {
        let bpe = Self::bits_per_entry(palette_size);
        (palette_size as u32) | ((bpe as u32) << 16)
    }
}

// ─── Chunk data container ───────────────────────────────────────────────

/// All CPU-side data needed to upload one chunk.
pub struct ChunkData {
    pub coord: ChunkCoord,
    pub occupancy: OccupancyBuilder,
    pub palette: PaletteBuilder,
    pub index_buf: IndexBufBuilder,
}

// ─── Material table entries ─────────────────────────────────────────────

/// A CPU-side material entry matching the GPU layout (4 × u32 packed f16 pairs).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MaterialEntry {
    pub albedo_rg: u32,
    pub albedo_b_roughness: u32,
    pub emissive_rg: u32,
    pub emissive_b_opacity: u32,
}

impl MaterialEntry {
    /// Create a material from RGB albedo, roughness, RGB emissive, and opacity.
    /// All values in [0.0, 1.0] (emissive can exceed 1.0 for HDR).
    pub fn new(
        albedo: [f32; 3],
        roughness: f32,
        emissive: [f32; 3],
        opacity: f32,
    ) -> Self {
        Self {
            albedo_rg: pack_f16_pair(albedo[0], albedo[1]),
            albedo_b_roughness: pack_f16_pair(albedo[2], roughness),
            emissive_rg: pack_f16_pair(emissive[0], emissive[1]),
            emissive_b_opacity: pack_f16_pair(emissive[2], opacity),
        }
    }
}

/// Pack two f32 values into a u32 as two f16 values.
fn pack_f16_pair(a: f32, b: f32) -> u32 {
    let a16 = f32_to_f16(a);
    let b16 = f32_to_f16(b);
    (a16 as u32) | ((b16 as u32) << 16)
}

/// Convert f32 to f16 (IEEE 754 half-precision). Truncates, no rounding.
fn f32_to_f16(val: f32) -> u16 {
    let bits = val.to_bits();
    let sign = (bits >> 16) & 0x8000;
    let exponent = ((bits >> 23) & 0xFF) as i32;
    let mantissa = bits & 0x7FFFFF;

    if exponent == 0xFF {
        // Inf/NaN
        return (sign | 0x7C00 | if mantissa != 0 { 0x0200 } else { 0 }) as u16;
    }

    let new_exp = exponent - 127 + 15;
    if new_exp >= 31 {
        // Overflow → Inf
        return (sign | 0x7C00) as u16;
    }
    if new_exp <= 0 {
        // Underflow → 0
        return sign as u16;
    }

    (sign | ((new_exp as u32) << 10) | (mantissa >> 13)) as u16
}

// ─── Procedural generators ──────────────────────────────────────────────

/// Material IDs used by the test scene.
pub const MAT_STONE: u16 = 2;
pub const MAT_BLUE: u16 = 3;
pub const MAT_EMISSIVE: u16 = 4;

// Cornell box materials
pub const MAT_WHITE: u16 = 5;
pub const MAT_RED: u16 = 6;
pub const MAT_GREEN: u16 = 7;
pub const MAT_LIGHT: u16 = 8;       // bright ceiling light
pub const MAT_GOLD: u16 = 9;        // smooth metallic-look
pub const MAT_ROUGH_GRAY: u16 = 10; // rough matte

/// Build the global material table for the test scene.
pub fn test_scene_materials() -> Vec<MaterialEntry> {
    let mut table = vec![MaterialEntry::new([0.0; 3], 0.0, [0.0; 3], 0.0); MAX_MATERIALS as usize];

    // 0: empty (already zeroed)
    // 1: default
    table[MATERIAL_DEFAULT as usize] =
        MaterialEntry::new([0.5, 0.5, 0.5], 0.5, [0.0; 3], 1.0);
    // 2: stone gray (room walls/floor)
    table[MAT_STONE as usize] =
        MaterialEntry::new([0.45, 0.43, 0.40], 0.8, [0.0; 3], 1.0);
    // 3: blue (sphere)
    table[MAT_BLUE as usize] =
        MaterialEntry::new([0.2, 0.35, 0.7], 0.4, [0.0; 3], 1.0);
    // 4: emissive yellow
    table[MAT_EMISSIVE as usize] =
        MaterialEntry::new([1.0, 0.9, 0.3], 0.2, [2.0, 1.8, 0.5], 1.0);

    table
}

/// Build the material table for the Cornell box test scene.
pub fn cornell_box_materials() -> Vec<MaterialEntry> {
    let mut table = vec![MaterialEntry::new([0.0; 3], 0.0, [0.0; 3], 0.0); MAX_MATERIALS as usize];

    table[MATERIAL_DEFAULT as usize] = MaterialEntry::new([0.5, 0.5, 0.5], 0.5, [0.0; 3], 1.0);
    // White walls/floor/ceiling
    table[MAT_WHITE as usize] = MaterialEntry::new([0.73, 0.73, 0.73], 0.9, [0.0; 3], 1.0);
    // Red left wall
    table[MAT_RED as usize] = MaterialEntry::new([0.65, 0.05, 0.05], 0.9, [0.0; 3], 1.0);
    // Green right wall
    table[MAT_GREEN as usize] = MaterialEntry::new([0.12, 0.45, 0.15], 0.9, [0.0; 3], 1.0);
    // Ceiling light — bright emissive
    table[MAT_LIGHT as usize] = MaterialEntry::new([1.0, 1.0, 1.0], 0.1, [8.0, 7.0, 5.0], 1.0);
    // Gold — smooth, warm
    table[MAT_GOLD as usize] = MaterialEntry::new([0.83, 0.69, 0.22], 0.15, [0.0; 3], 1.0);
    // Rough gray — matte concrete look
    table[MAT_ROUGH_GRAY as usize] = MaterialEntry::new([0.4, 0.4, 0.4], 0.95, [0.0; 3], 1.0);

    table
}

/// Generate a Cornell box test scene: colored walls, ceiling light, two objects.
/// Classic GI test — color bleeding from walls proves indirect illumination.
pub fn generate_cornell_box() -> (Vec<ChunkData>, Vec<MaterialEntry>) {
    let coord = ChunkCoord { x: 0, y: 0, z: 0 };
    let mut occ = OccupancyBuilder::new();
    let mut pal = PaletteBuilder::new();
    let mut idx = IndexBufBuilder::new();

    let white_idx = pal.add(MAT_WHITE);
    let red_idx = pal.add(MAT_RED);
    let green_idx = pal.add(MAT_GREEN);
    let light_idx = pal.add(MAT_LIGHT);
    let gold_idx = pal.add(MAT_GOLD);
    let rough_idx = pal.add(MAT_ROUGH_GRAY);

    let lo = 1u32;
    let hi = CS_P - 2; // 62

    // ── Walls, floor, ceiling ──
    for x in lo..=hi {
        for z in lo..=hi {
            // Floor (white)
            occ.set(x, lo, z);
            idx.set(x, lo, z, white_idx);
            // Ceiling (white)
            occ.set(x, hi, z);
            idx.set(x, hi, z, white_idx);
        }
    }

    for y in lo..=hi {
        for z in lo..=hi {
            // Left wall (RED)
            occ.set(lo, y, z);
            idx.set(lo, y, z, red_idx);
            // Right wall (GREEN)
            occ.set(hi, y, z);
            idx.set(hi, y, z, green_idx);
        }
        for x in lo..=hi {
            // Back wall (white)
            occ.set(x, y, lo);
            idx.set(x, y, lo, white_idx);
            // Front wall open (no wall — camera looks in from here)
        }
    }

    // ── Ceiling light (emissive panel, centered, ~20×20) ──
    let light_lo = 22u32;
    let light_hi = 42u32;
    for x in light_lo..=light_hi {
        for z in light_lo..=light_hi {
            // Overwrite ceiling voxels with emissive
            idx.set(x, hi, z, light_idx);
        }
    }

    // ── Tall gold box (left side) ──
    let box_x = (12u32, 26u32);
    let box_y = (lo + 1, 35u32);
    let box_z = (15u32, 29u32);
    for x in box_x.0..=box_x.1 {
        for y in box_y.0..=box_y.1 {
            for z in box_z.0..=box_z.1 {
                occ.set(x, y, z);
                idx.set(x, y, z, gold_idx);
            }
        }
    }

    // ── Short rough cube (right side) ──
    let cube_x = (36u32, 50u32);
    let cube_y = (lo + 1, 18u32);
    let cube_z = (32u32, 46u32);
    for x in cube_x.0..=cube_x.1 {
        for y in cube_y.0..=cube_y.1 {
            for z in cube_z.0..=cube_z.1 {
                occ.set(x, y, z);
                idx.set(x, y, z, rough_idx);
            }
        }
    }

    let chunk = ChunkData {
        coord,
        occupancy: occ,
        palette: pal,
        index_buf: idx,
    };

    let materials = cornell_box_materials();
    (vec![chunk], materials)
}

/// Generate a room chunk: floor, ceiling, and four walls.
/// Occupies the usable interior [1..62] of a 64³ padded chunk.
pub fn generate_room(coord: ChunkCoord) -> ChunkData {
    let mut occ = OccupancyBuilder::new();
    let mut pal = PaletteBuilder::new();
    let mut idx = IndexBufBuilder::new();
    let stone_idx = pal.add(MAT_STONE);

    let lo = 1u32; // first usable voxel
    let hi = CS_P - 2; // last usable voxel (62)

    for x in lo..=hi {
        for z in lo..=hi {
            // Floor
            occ.set(x, lo, z);
            idx.set(x, lo, z, stone_idx);
            // Ceiling
            occ.set(x, hi, z);
            idx.set(x, hi, z, stone_idx);
        }
    }

    for y in lo..=hi {
        for x in lo..=hi {
            // Front wall (z = lo)
            occ.set(x, y, lo);
            idx.set(x, y, lo, stone_idx);
            // Back wall (z = hi)
            occ.set(x, y, hi);
            idx.set(x, y, hi, stone_idx);
        }
        for z in lo..=hi {
            // Left wall (x = lo)
            occ.set(lo, y, z);
            idx.set(lo, y, z, stone_idx);
            // Right wall (x = hi)
            occ.set(hi, y, z);
            idx.set(hi, y, z, stone_idx);
        }
    }

    ChunkData {
        coord,
        occupancy: occ,
        palette: pal,
        index_buf: idx,
    }
}

/// Generate a solid sphere centered at (cx, cy, cz) with given radius.
pub fn generate_sphere(
    coord: ChunkCoord,
    cx: u32,
    cy: u32,
    cz: u32,
    radius: u32,
) -> ChunkData {
    let mut occ = OccupancyBuilder::new();
    let mut pal = PaletteBuilder::new();
    let mut idx = IndexBufBuilder::new();
    let blue_idx = pal.add(MAT_BLUE);

    let r2 = (radius * radius) as i64;
    let lo = 1u32;
    let hi = CS_P - 2;

    for x in lo..=hi {
        for y in lo..=hi {
            for z in lo..=hi {
                let dx = x as i64 - cx as i64;
                let dy = y as i64 - cy as i64;
                let dz = z as i64 - cz as i64;
                if dx * dx + dy * dy + dz * dz <= r2 {
                    occ.set(x, y, z);
                    idx.set(x, y, z, blue_idx);
                }
            }
        }
    }

    ChunkData {
        coord,
        occupancy: occ,
        palette: pal,
        index_buf: idx,
    }
}

/// Generate a few emissive voxel clusters at fixed positions within a chunk.
pub fn generate_emissive_clusters(coord: ChunkCoord) -> ChunkData {
    let mut occ = OccupancyBuilder::new();
    let mut pal = PaletteBuilder::new();
    let mut idx = IndexBufBuilder::new();
    let emissive_idx = pal.add(MAT_EMISSIVE);

    // Three 2×2×2 clusters at different positions
    let clusters: [(u32, u32, u32); 3] = [
        (10, 8, 10),
        (50, 15, 30),
        (30, 10, 50),
    ];

    for &(bx, by, bz) in &clusters {
        for dx in 0..2 {
            for dy in 0..2 {
                for dz in 0..2 {
                    occ.set(bx + dx, by + dy, bz + dz);
                    idx.set(bx + dx, by + dy, bz + dz, emissive_idx);
                }
            }
        }
    }

    ChunkData {
        coord,
        occupancy: occ,
        palette: pal,
        index_buf: idx,
    }
}

/// Generate the complete test scene: room + sphere + emissive clusters.
/// All placed in chunk (0, 0, 0). The sphere and emissive voxels are merged
/// into the room chunk's occupancy. Overlapping voxels take the later layer's
/// material (sphere overwrites room stone, emissive overwrites both).
pub fn generate_test_scene() -> (Vec<ChunkData>, Vec<MaterialEntry>) {
    let coord = ChunkCoord { x: 0, y: 0, z: 0 };

    // Start with the room
    let mut room = generate_room(coord);

    // Add sphere into the same occupancy — sphere's blue overwrites room's stone
    let sphere = generate_sphere(coord, 32, 25, 32, 12);
    let blue_idx = room.palette.add(MAT_BLUE);
    merge_chunk_layer(&mut room, &sphere, blue_idx);

    // Add emissive clusters — emissive overwrites everything
    let emissive = generate_emissive_clusters(coord);
    let emissive_idx = room.palette.add(MAT_EMISSIVE);
    merge_chunk_layer(&mut room, &emissive, emissive_idx);

    let materials = test_scene_materials();
    (vec![room], materials)
}

/// Merge source occupancy into dest. Where source has voxels, dest gets
/// those voxels with the specified palette index (overwriting any existing material).
fn merge_chunk_layer(dest: &mut ChunkData, src: &ChunkData, palette_idx: u8) {
    for x in 0..CS_P {
        for y in 0..CS_P {
            for z in 0..CS_P {
                if src.occupancy.get(x, y, z) {
                    dest.occupancy.set(x, y, z);
                    dest.index_buf.set(x, y, z, palette_idx);
                }
            }
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occupancy_builder_set_get() {
        let mut b = OccupancyBuilder::new();
        assert!(!b.get(10, 20, 30));
        b.set(10, 20, 30);
        assert!(b.get(10, 20, 30));
        assert!(!b.get(10, 20, 31));
        assert!(!b.get(10, 21, 30));
        assert!(!b.get(11, 20, 30));
    }

    #[test]
    fn occupancy_builder_clear() {
        let mut b = OccupancyBuilder::new();
        b.set(5, 5, 5);
        assert!(b.get(5, 5, 5));
        b.clear(5, 5, 5);
        assert!(!b.get(5, 5, 5));
    }

    #[test]
    fn occupancy_builder_corners() {
        let mut b = OccupancyBuilder::new();
        // All 8 corners of the 64³ grid
        let corners = [
            (0, 0, 0), (63, 0, 0), (0, 63, 0), (0, 0, 63),
            (63, 63, 0), (63, 0, 63), (0, 63, 63), (63, 63, 63),
        ];
        for &(x, y, z) in &corners {
            b.set(x, y, z);
        }
        for &(x, y, z) in &corners {
            assert!(b.get(x, y, z), "Corner ({x},{y},{z}) not set");
        }
        assert_eq!(b.popcount(), 8);
    }

    #[test]
    fn occupancy_builder_popcount() {
        let mut b = OccupancyBuilder::new();
        assert_eq!(b.popcount(), 0);
        b.set(0, 0, 0);
        b.set(1, 1, 1);
        b.set(2, 2, 2);
        assert_eq!(b.popcount(), 3);
    }

    #[test]
    fn occupancy_builder_word_count() {
        let b = OccupancyBuilder::new();
        assert_eq!(b.as_words().len(), OCCUPANCY_WORDS_PER_SLOT as usize);
    }

    #[test]
    fn occupancy_builder_y_boundary() {
        // Test voxels at y=31 and y=32 (u32 word boundary)
        let mut b = OccupancyBuilder::new();
        b.set(0, 31, 0);
        b.set(0, 32, 0);
        assert!(b.get(0, 31, 0));
        assert!(b.get(0, 32, 0));
        assert!(!b.get(0, 30, 0));
        assert!(!b.get(0, 33, 0));
    }

    #[test]
    fn palette_builder_basics() {
        let mut p = PaletteBuilder::new();
        assert_eq!(p.len(), 1); // starts with MATERIAL_EMPTY
        let idx = p.add(MAT_STONE);
        assert_eq!(idx, 1);
        // Adding same material returns same index
        let idx2 = p.add(MAT_STONE);
        assert_eq!(idx2, 1);
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn palette_builder_packing() {
        let mut p = PaletteBuilder::new();
        p.add(MAT_STONE); // index 1
        p.add(MAT_BLUE);  // index 2
        let words = p.as_words();
        // Word 0: entries[0] (lo 16) | entries[1] (hi 16)
        assert_eq!(words[0] & 0xFFFF, MATERIAL_EMPTY as u32);
        assert_eq!((words[0] >> 16) & 0xFFFF, MAT_STONE as u32);
        // Word 1: entries[2] (lo 16)
        assert_eq!(words[1] & 0xFFFF, MAT_BLUE as u32);
    }

    #[test]
    fn f16_roundtrip_common_values() {
        // Verify f16 conversion doesn't lose significant precision for [0, 1] range
        for &val in &[0.0f32, 0.5, 1.0, 0.25, 0.75] {
            let packed = pack_f16_pair(val, 0.0);
            let f16_bits = (packed & 0xFFFF) as u16;
            // Reconstruct: just verify it's non-zero for non-zero input
            if val > 0.0 {
                assert_ne!(f16_bits, 0, "f16 of {val} should be non-zero");
            } else {
                assert_eq!(f16_bits, 0);
            }
        }
    }

    #[test]
    fn generate_room_has_walls_floor_ceiling() {
        let chunk = generate_room(ChunkCoord { x: 0, y: 0, z: 0 });
        let occ = &chunk.occupancy;

        // Floor: y=1 should be filled across interior
        assert!(occ.get(10, 1, 10));
        assert!(occ.get(50, 1, 50));
        // Ceiling: y=62
        assert!(occ.get(10, 62, 10));
        // Walls
        assert!(occ.get(1, 30, 30));  // left
        assert!(occ.get(62, 30, 30)); // right
        assert!(occ.get(30, 30, 1));  // front
        assert!(occ.get(30, 30, 62)); // back
        // Interior should be empty
        assert!(!occ.get(30, 30, 30));

        assert!(occ.popcount() > 0);
    }

    #[test]
    fn generate_sphere_shape() {
        let chunk = generate_sphere(ChunkCoord { x: 0, y: 0, z: 0 }, 32, 25, 32, 10);
        let occ = &chunk.occupancy;

        // Center should be occupied
        assert!(occ.get(32, 25, 32));
        // Point on surface (radius = 10 along x)
        assert!(occ.get(42, 25, 32));
        // Point outside (radius + 2)
        assert!(!occ.get(44, 25, 32));

        // Rough volume check: 4/3 π r³ ≈ 4189 for r=10
        let count = occ.popcount();
        assert!(count > 3000, "Sphere too small: {count} voxels");
        assert!(count < 5500, "Sphere too large: {count} voxels");
    }

    #[test]
    fn generate_emissive_clusters_count() {
        let chunk = generate_emissive_clusters(ChunkCoord { x: 0, y: 0, z: 0 });
        // 3 clusters × 2³ = 24 voxels
        assert_eq!(chunk.occupancy.popcount(), 24);
    }

    #[test]
    fn test_scene_is_nonempty() {
        let (chunks, materials) = generate_test_scene();
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].occupancy.popcount() > 0);
        assert_eq!(materials.len(), MAX_MATERIALS as usize);

        // Verify materials are populated
        let stone = &materials[MAT_STONE as usize];
        assert_ne!(stone.albedo_rg, 0);
        let emissive = &materials[MAT_EMISSIVE as usize];
        assert_ne!(emissive.emissive_rg, 0);
    }

    #[test]
    fn merge_chunk_layer_is_union() {
        let coord = ChunkCoord { x: 0, y: 0, z: 0 };
        let mut dest = ChunkData {
            coord,
            occupancy: OccupancyBuilder::new(),
            palette: PaletteBuilder::new(),
            index_buf: IndexBufBuilder::new(),
        };
        let mut src = ChunkData {
            coord,
            occupancy: OccupancyBuilder::new(),
            palette: PaletteBuilder::new(),
            index_buf: IndexBufBuilder::new(),
        };
        dest.occupancy.set(10, 10, 10);
        let mat_a = dest.palette.add(MAT_STONE);
        dest.index_buf.set(10, 10, 10, mat_a);

        src.occupancy.set(20, 20, 20);

        let mat_b = dest.palette.add(MAT_BLUE);
        merge_chunk_layer(&mut dest, &src, mat_b);

        assert!(dest.occupancy.get(10, 10, 10));
        assert!(dest.occupancy.get(20, 20, 20));
        assert_eq!(dest.occupancy.popcount(), 2);
        // Material assignments are correct
        assert_eq!(dest.index_buf.get(10, 10, 10), mat_a);
        assert_eq!(dest.index_buf.get(20, 20, 20), mat_b);
    }

    #[test]
    fn material_entry_size() {
        assert_eq!(
            std::mem::size_of::<MaterialEntry>(),
            MATERIAL_ENTRY_BYTES as usize,
            "MaterialEntry must be exactly 16 bytes"
        );
    }

    // ── IndexBufBuilder tests ──

    #[test]
    fn index_buf_set_get() {
        let mut ib = IndexBufBuilder::new();
        assert_eq!(ib.get(10, 20, 30), 0);
        ib.set(10, 20, 30, 5);
        assert_eq!(ib.get(10, 20, 30), 5);
        assert_eq!(ib.get(10, 20, 31), 0); // neighbor unchanged
    }

    #[test]
    fn index_buf_bits_per_entry() {
        assert_eq!(IndexBufBuilder::bits_per_entry(1), 1);
        assert_eq!(IndexBufBuilder::bits_per_entry(2), 1);
        assert_eq!(IndexBufBuilder::bits_per_entry(3), 2);
        assert_eq!(IndexBufBuilder::bits_per_entry(4), 2);
        assert_eq!(IndexBufBuilder::bits_per_entry(5), 4);
        assert_eq!(IndexBufBuilder::bits_per_entry(16), 4);
        assert_eq!(IndexBufBuilder::bits_per_entry(17), 8);
        assert_eq!(IndexBufBuilder::bits_per_entry(256), 8);
    }

    #[test]
    fn index_buf_pack_roundtrip_bpe1() {
        let mut ib = IndexBufBuilder::new();
        ib.set(1, 1, 1, 1);
        ib.set(32, 32, 32, 0);
        ib.set(62, 62, 62, 1);
        let packed = ib.pack(1);
        // Decode and verify
        for x in 0..CS_P {
            for y in 0..CS_P {
                for z in 0..CS_P {
                    let expected = ib.get(x, y, z);
                    let vi = (x * CS_P * CS_P + y * CS_P + z) as usize;
                    let word = packed[vi / 32];
                    let bit = vi % 32;
                    let decoded = ((word >> bit) & 1) as u8;
                    assert_eq!(decoded, expected, "mismatch at ({x},{y},{z})");
                }
            }
        }
    }

    #[test]
    fn index_buf_pack_roundtrip_bpe8() {
        let mut ib = IndexBufBuilder::new();
        ib.set(1, 1, 1, 42);
        ib.set(10, 20, 30, 255);
        ib.set(62, 62, 62, 7);
        let packed = ib.pack(8);
        // Decode and verify selected entries
        let decode = |x: u32, y: u32, z: u32| -> u8 {
            let vi = (x * CS_P * CS_P + y * CS_P + z) as usize;
            let bit_off = vi * 8;
            let word = packed[bit_off / 32];
            let bit = bit_off % 32;
            ((word >> bit) & 0xFF) as u8
        };
        assert_eq!(decode(1, 1, 1), 42);
        assert_eq!(decode(10, 20, 30), 255);
        assert_eq!(decode(62, 62, 62), 7);
        assert_eq!(decode(0, 0, 0), 0); // unset voxel
    }

    #[test]
    fn index_buf_pack_roundtrip_bpe4() {
        let mut ib = IndexBufBuilder::new();
        // Set various values in [0, 15] range
        ib.set(5, 5, 5, 12);
        ib.set(5, 5, 6, 3);
        ib.set(63, 63, 63, 15);
        let packed = ib.pack(4);
        let decode = |x: u32, y: u32, z: u32| -> u8 {
            let vi = (x * CS_P * CS_P + y * CS_P + z) as usize;
            let bit_off = vi * 4;
            let word = packed[bit_off / 32];
            let bit = bit_off % 32;
            ((word >> bit) & 0xF) as u8
        };
        assert_eq!(decode(5, 5, 5), 12);
        assert_eq!(decode(5, 5, 6), 3);
        assert_eq!(decode(63, 63, 63), 15);
        assert_eq!(decode(0, 0, 0), 0);
    }

    #[test]
    fn index_buf_palette_meta_packing() {
        let meta = IndexBufBuilder::palette_meta(5);
        let palette_size = meta & 0xFFFF;
        let bpe = (meta >> 16) & 0xFF;
        let reserved = (meta >> 24) & 0xFF;
        assert_eq!(palette_size, 5);
        assert_eq!(bpe, 4); // 5 entries → bpe=4
        assert_eq!(reserved, 0); // IDX-5: reserved bits must be 0
    }
}
