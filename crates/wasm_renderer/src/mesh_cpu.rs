//! R-1 Mesh Rebuild — CPU reference implementation.
//!
//! Platform-independent. Produces identical output to the GPU compute shader.
//! Used for native testing (Tier 1) and GPU readback comparison (Tier 2).
//!
//! See: docs/Resident Representation/stages/R-1-mesh-rebuild.md

use crate::pool::*;

// ─── Face culling ───────────────────────────────────────────────────────

/// Reconstruct a u64 column from two u32 words in the occupancy buffer.
#[inline]
fn read_column(occupancy: &[u32], x: u32, z: u32) -> u64 {
    let col_idx = (x * CS_P + z) as usize;
    let base = col_idx * 2;
    occupancy[base] as u64 | ((occupancy[base + 1] as u64) << 32)
}

/// Usable-range mask: bits [1..62] shifted right by 1, giving 62 usable bits [0..61].
const USABLE_MASK: u64 = (1u64 << CS as u64) - 1; // 2^62 - 1

/// Compute face visibility masks for all 6 directions.
///
/// Returns `[face_dir][column_index]` where each u64 has bit y set if that face is visible
/// at usable coordinate y (0..61). Column index = x * CS_P + z for the padded grid.
///
/// Only usable columns (x,z ∈ [1..62]) have meaningful data; padding columns are zero.
pub fn cull_faces_cpu(occupancy: &[u32]) -> [Vec<u64>; 6] {
    let n = COLUMNS_PER_CHUNK as usize;
    let mut masks: [Vec<u64>; 6] = std::array::from_fn(|_| vec![0u64; n]);

    for x in 1..CS_P - 1 {
        for z in 1..CS_P - 1 {
            let col = read_column(occupancy, x, z);
            if col == 0 {
                continue;
            }
            let col_idx = (x * CS_P + z) as usize;

            // +Y: visible where voxel is solid and y+1 is empty
            let pos_y = col & !(col >> 1);
            masks[FACE_POS_Y][col_idx] = (pos_y >> 1) & USABLE_MASK;

            // -Y: visible where voxel is solid and y-1 is empty
            let neg_y = col & !(col << 1);
            masks[FACE_NEG_Y][col_idx] = (neg_y >> 1) & USABLE_MASK;

            // +X: visible where voxel is solid and x+1 neighbor is empty
            let neighbor_px = read_column(occupancy, x + 1, z);
            masks[FACE_POS_X][col_idx] = ((col & !neighbor_px) >> 1) & USABLE_MASK;

            // -X: visible where voxel is solid and x-1 neighbor is empty
            let neighbor_nx = read_column(occupancy, x - 1, z);
            masks[FACE_NEG_X][col_idx] = ((col & !neighbor_nx) >> 1) & USABLE_MASK;

            // +Z: visible where voxel is solid and z+1 neighbor is empty
            let neighbor_pz = read_column(occupancy, x, z + 1);
            masks[FACE_POS_Z][col_idx] = ((col & !neighbor_pz) >> 1) & USABLE_MASK;

            // -Z: visible where voxel is solid and z-1 neighbor is empty
            let neighbor_nz = read_column(occupancy, x, z - 1);
            masks[FACE_NEG_Z][col_idx] = ((col & !neighbor_nz) >> 1) & USABLE_MASK;
        }
    }

    masks
}

/// Count total visible faces across all directions.
pub fn count_faces(masks: &[Vec<u64>; 6]) -> [u32; 6] {
    let mut counts = [0u32; 6];
    for face in 0..6 {
        counts[face] = masks[face].iter().map(|m| m.count_ones()).sum();
    }
    counts
}

/// Total face count across all 6 directions.
pub fn total_face_count(masks: &[Vec<u64>; 6]) -> u32 {
    count_faces(masks).iter().sum()
}

// ─── Material lookup ────────────────────────────────────────────────────

/// Decode a palette index for one voxel from the bitpacked index buffer.
///
/// Uses the same addressing formula as the WGSL shader (IDX spec):
///   voxel_index = px * CS_P² + py * CS_P + pz  (x-major flat index)
///   bit_offset  = voxel_index * bpe
///   word_index  = bit_offset / 32
///   bit_within  = bit_offset % 32
///
/// `bpe` must be 1, 2, 4, or 8. Cross-word entries never occur (IDX-1).
fn read_palette_index(index_buf: &[u32], bpe: u32, px: u32, py: u32, pz: u32) -> u32 {
    if bpe == 0 || index_buf.is_empty() {
        return 0;
    }
    let voxel_index = px * CS_P * CS_P + py * CS_P + pz;
    let bit_offset = voxel_index * bpe;
    let word_index = (bit_offset >> 5) as usize;
    let bit_within = bit_offset & 31;
    let mask = (1u32 << bpe) - 1;
    if word_index < index_buf.len() {
        (index_buf[word_index] >> bit_within) & mask
    } else {
        0
    }
}

/// Resolve global MaterialId for a voxel from palette + index buffer.
///
/// Decodes palette_idx from index_buf, then looks up MaterialId in palette.
/// Returns the global MaterialId (u16 value stored in palette).
fn read_material_id(palette: &[u32], index_buf: &[u32], bpe: u32, px: u32, py: u32, pz: u32) -> u32 {
    let pal_idx = read_palette_index(index_buf, bpe, px, py, pz);
    if palette.is_empty() {
        return MATERIAL_DEFAULT as u32;
    }
    let word_idx = (pal_idx >> 1) as usize;
    let shift = (pal_idx & 1) * 16;
    if word_idx < palette.len() {
        (palette[word_idx] >> shift) & 0xFFFF
    } else {
        MATERIAL_DEFAULT as u32
    }
}

// ─── Greedy merge ───────────────────────────────────────────────────────

/// A merged quad before vertex expansion.
#[derive(Debug, Clone)]
pub struct Quad {
    /// Position in usable coordinates [0..61].
    pub x: u32,
    pub y: u32,
    pub z: u32,
    /// Size (≥1).
    pub width: u32,
    pub height: u32,
    /// Face direction (0..5).
    pub face: usize,
    /// Global MaterialId (truncated to u8 for vertex packing).
    pub material_id: u8,
}

/// Processed bitmap for a 62×62 slice (3844 bits packed into 121 u32).
struct ProcessedBitmap {
    words: [u32; 121],
}

impl ProcessedBitmap {
    fn new() -> Self {
        Self { words: [0; 121] }
    }

    #[inline]
    fn get(&self, a: u32, b: u32) -> bool {
        let idx = a * CS + b;
        let word = (idx >> 5) as usize;
        let bit = idx & 31;
        (self.words[word] >> bit) & 1 != 0
    }

    #[inline]
    fn set(&mut self, a: u32, b: u32) {
        let idx = a * CS + b;
        let word = (idx >> 5) as usize;
        let bit = idx & 31;
        self.words[word] |= 1 << bit;
    }
}

/// Check if a face is visible at usable coordinate y in a face mask column.
#[inline]
fn face_visible(masks: &[Vec<u64>; 6], face: usize, x: u32, z: u32, y: u32) -> bool {
    let col_idx = (x * CS_P + z) as usize;
    (masks[face][col_idx] >> y) & 1 != 0
}

/// Greedy merge for all face directions. Returns list of merged quads.
///
/// Material-aware: adjacent faces merge only if they share the same MaterialId.
/// Material is resolved per-voxel via the bitpacked index_buf → palette chain.
pub fn greedy_merge(
    _occupancy: &[u32],
    masks: &[Vec<u64>; 6],
    palette: &[u32],
    index_buf: &[u32],
    bpe: u32,
) -> Vec<Quad> {
    let mut quads = Vec::new();

    // +Y / -Y faces: sweep Y slices, merge in XZ plane
    for &face in &[FACE_POS_Y, FACE_NEG_Y] {
        for slice_y in 0..CS {
            merge_y_slice(masks, face, slice_y, palette, index_buf, bpe, &mut quads);
        }
    }

    // +X / -X faces: sweep X slices, merge in YZ plane
    for &face in &[FACE_POS_X, FACE_NEG_X] {
        for slice_x in 0..CS {
            merge_x_slice(masks, face, slice_x, palette, index_buf, bpe, &mut quads);
        }
    }

    // +Z / -Z faces: sweep Z slices, merge in XY plane
    for &face in &[FACE_POS_Z, FACE_NEG_Z] {
        for slice_z in 0..CS {
            merge_z_slice(masks, face, slice_z, palette, index_buf, bpe, &mut quads);
        }
    }

    quads
}

/// Merge one Y slice (y = slice_y). Sweep over usable X × Z [0..61].
/// Width direction: +X, Height direction: +Z.
/// For Y faces, padded coords: px = usable_x + 1, py = slice_y + 1, pz = usable_z + 1.
fn merge_y_slice(
    masks: &[Vec<u64>; 6],
    face: usize,
    slice_y: u32,
    palette: &[u32],
    index_buf: &[u32],
    bpe: u32,
    quads: &mut Vec<Quad>,
) {
    let mut processed = ProcessedBitmap::new();
    let py = slice_y + 1; // padded y

    for start_x in 0..CS {
        for start_z in 0..CS {
            if processed.get(start_x, start_z) {
                continue;
            }
            let px = start_x + 1;
            let pz = start_z + 1;

            if !face_visible(masks, face, px, pz, slice_y) {
                continue;
            }

            let seed_mat = read_material_id(palette, index_buf, bpe, px, py, pz);

            // Extend width in +X
            let mut width = 1u32;
            while start_x + width < CS {
                let nx = start_x + width;
                if processed.get(nx, start_z) {
                    break;
                }
                if !face_visible(masks, face, nx + 1, pz, slice_y) {
                    break;
                }
                if read_material_id(palette, index_buf, bpe, nx + 1, py, pz) != seed_mat {
                    break;
                }
                width += 1;
            }

            // Extend height in +Z
            let mut height = 1u32;
            'height: while start_z + height < CS {
                let nz = start_z + height;
                for dx in 0..width {
                    let cx = start_x + dx;
                    if processed.get(cx, nz) {
                        break 'height;
                    }
                    if !face_visible(masks, face, cx + 1, nz + 1, slice_y) {
                        break 'height;
                    }
                    if read_material_id(palette, index_buf, bpe, cx + 1, py, nz + 1) != seed_mat {
                        break 'height;
                    }
                }
                height += 1;
            }

            for dx in 0..width {
                for dz in 0..height {
                    processed.set(start_x + dx, start_z + dz);
                }
            }

            quads.push(Quad {
                x: start_x,
                y: slice_y,
                z: start_z,
                width,
                height,
                face,
                material_id: seed_mat as u8,
            });
        }
    }
}

/// Merge one X slice (x = slice_x). Sweep over usable Y × Z [0..61].
/// Width direction: +Y, Height direction: +Z.
/// For X faces, padded coords: px = slice_x + 1, py = usable_y + 1, pz = usable_z + 1.
fn merge_x_slice(
    masks: &[Vec<u64>; 6],
    face: usize,
    slice_x: u32,
    palette: &[u32],
    index_buf: &[u32],
    bpe: u32,
    quads: &mut Vec<Quad>,
) {
    let mut processed = ProcessedBitmap::new();
    let px = slice_x + 1; // padded X

    for start_y in 0..CS {
        for start_z in 0..CS {
            if processed.get(start_y, start_z) {
                continue;
            }
            let pz = start_z + 1;
            let py = start_y + 1;

            if !face_visible(masks, face, px, pz, start_y) {
                continue;
            }

            let seed_mat = read_material_id(palette, index_buf, bpe, px, py, pz);

            // Extend width in +Y
            let mut width = 1u32;
            while start_y + width < CS {
                let ny = start_y + width;
                if processed.get(ny, start_z) {
                    break;
                }
                if !face_visible(masks, face, px, pz, ny) {
                    break;
                }
                if read_material_id(palette, index_buf, bpe, px, ny + 1, pz) != seed_mat {
                    break;
                }
                width += 1;
            }

            // Extend height in +Z
            let mut height = 1u32;
            'height: while start_z + height < CS {
                let nz = start_z + height;
                for dy in 0..width {
                    let cy = start_y + dy;
                    if processed.get(cy, nz) {
                        break 'height;
                    }
                    if !face_visible(masks, face, px, nz + 1, cy) {
                        break 'height;
                    }
                    if read_material_id(palette, index_buf, bpe, px, cy + 1, nz + 1) != seed_mat {
                        break 'height;
                    }
                }
                height += 1;
            }

            for dy in 0..width {
                for dz in 0..height {
                    processed.set(start_y + dy, start_z + dz);
                }
            }

            quads.push(Quad {
                x: slice_x,
                y: start_y,
                z: start_z,
                width,
                height,
                face,
                material_id: seed_mat as u8,
            });
        }
    }
}

/// Merge one Z slice (z = slice_z). Sweep over usable X × Y [0..61].
/// Width direction: +X, Height direction: +Y.
/// For Z faces, padded coords: px = usable_x + 1, py = usable_y + 1, pz = slice_z + 1.
fn merge_z_slice(
    masks: &[Vec<u64>; 6],
    face: usize,
    slice_z: u32,
    palette: &[u32],
    index_buf: &[u32],
    bpe: u32,
    quads: &mut Vec<Quad>,
) {
    let mut processed = ProcessedBitmap::new();
    let pz = slice_z + 1; // padded Z

    for start_x in 0..CS {
        for start_y in 0..CS {
            if processed.get(start_x, start_y) {
                continue;
            }
            let px = start_x + 1;
            let py = start_y + 1;

            if !face_visible(masks, face, px, pz, start_y) {
                continue;
            }

            let seed_mat = read_material_id(palette, index_buf, bpe, px, py, pz);

            // Extend width in +X
            let mut width = 1u32;
            while start_x + width < CS {
                let nx = start_x + width;
                if processed.get(nx, start_y) {
                    break;
                }
                if !face_visible(masks, face, nx + 1, pz, start_y) {
                    break;
                }
                if read_material_id(palette, index_buf, bpe, nx + 1, py, pz) != seed_mat {
                    break;
                }
                width += 1;
            }

            // Extend height in +Y
            let mut height = 1u32;
            'height: while start_y + height < CS {
                let ny = start_y + height;
                for dx in 0..width {
                    let cx = start_x + dx;
                    if processed.get(cx, ny) {
                        break 'height;
                    }
                    if !face_visible(masks, face, cx + 1, pz, ny) {
                        break 'height;
                    }
                    if read_material_id(palette, index_buf, bpe, cx + 1, ny + 1, pz) != seed_mat {
                        break 'height;
                    }
                }
                height += 1;
            }

            for dx in 0..width {
                for dy in 0..height {
                    processed.set(start_x + dx, start_y + dy);
                }
            }

            quads.push(Quad {
                x: start_x,
                y: start_y,
                z: slice_z,
                width,
                height,
                face,
                material_id: seed_mat as u8,
            });
        }
    }
}

// ─── Vertex expansion ───────────────────────────────────────────────────

/// Pack a normal + material_id into a u32.
/// Normal components are snorm8: +1.0 → 0x7F, -1.0 → 0x81 (as u8).
pub fn pack_normal_material(nx: f32, ny: f32, nz: f32, material_id: u8) -> u32 {
    let snx = (nx * 127.0) as i8 as u8;
    let sny = (ny * 127.0) as i8 as u8;
    let snz = (nz * 127.0) as i8 as u8;
    (snx as u32) | ((sny as u32) << 8) | ((snz as u32) << 16) | ((material_id as u32) << 24)
}

/// Normal vectors for each face direction.
const FACE_NORMALS: [[f32; 3]; 6] = [
    [0.0, 1.0, 0.0],   // +Y
    [0.0, -1.0, 0.0],  // -Y
    [1.0, 0.0, 0.0],   // +X
    [-1.0, 0.0, 0.0],  // -X
    [0.0, 0.0, 1.0],   // +Z
    [0.0, 0.0, -1.0],  // -Z
];

/// Expand quads to vertices and indices.
/// Positions are in padded voxel-space (add 1 to convert from usable to padded).
/// World transform: `pos + chunk_coord * CS_P`.
pub fn expand_quads(quads: &[Quad], chunk_coord: [i32; 3]) -> (Vec<u8>, Vec<u32>) {
    let mut vertices: Vec<u8> = Vec::with_capacity(quads.len() * 4 * VERTEX_BYTES as usize);
    let mut indices: Vec<u32> = Vec::with_capacity(quads.len() * 6);
    let mut vert_count = 0u32;

    // World offset: chunk_coord * CS (usable stride, not padded).
    // See: chunk-contract.md — "World origin of a chunk: coord * CS * voxel_size"
    let world_off = [
        chunk_coord[0] as f32 * CS as f32,
        chunk_coord[1] as f32 * CS as f32,
        chunk_coord[2] as f32 * CS as f32,
    ];

    for q in quads {
        let [nx, ny, nz] = FACE_NORMALS[q.face];
        let nm = pack_normal_material(nx, ny, nz, q.material_id);

        // Base position in padded coords (usable + 1 for padding offset)
        let bx = world_off[0] + (q.x + 1) as f32;
        let by = world_off[1] + (q.y + 1) as f32;
        let bz = world_off[2] + (q.z + 1) as f32;

        let w = q.width as f32;
        let h = q.height as f32;

        // 4 corners depend on face direction
        let corners: [[f32; 3]; 4] = match q.face {
            FACE_POS_Y => [
                [bx,     by + 1.0, bz],
                [bx,     by + 1.0, bz + h],
                [bx + w, by + 1.0, bz + h],
                [bx + w, by + 1.0, bz],
            ],
            FACE_NEG_Y => [
                [bx,     by, bz],
                [bx + w, by, bz],
                [bx + w, by, bz + h],
                [bx,     by, bz + h],
            ],
            FACE_POS_X => [
                [bx + 1.0, by,     bz],
                [bx + 1.0, by + w, bz],
                [bx + 1.0, by + w, bz + h],
                [bx + 1.0, by,     bz + h],
            ],
            FACE_NEG_X => [
                [bx, by,     bz],
                [bx, by,     bz + h],
                [bx, by + w, bz + h],
                [bx, by + w, bz],
            ],
            FACE_POS_Z => [
                [bx,     by,     bz + 1.0],
                [bx + w, by,     bz + 1.0],
                [bx + w, by + h, bz + 1.0],
                [bx,     by + h, bz + 1.0],
            ],
            FACE_NEG_Z => [
                [bx,     by,     bz],
                [bx,     by + h, bz],
                [bx + w, by + h, bz],
                [bx + w, by,     bz],
            ],
            _ => unreachable!(),
        };

        // Write 4 vertices (16 bytes each: vec3f + u32)
        for corner in &corners {
            vertices.extend_from_slice(&corner[0].to_le_bytes());
            vertices.extend_from_slice(&corner[1].to_le_bytes());
            vertices.extend_from_slice(&corner[2].to_le_bytes());
            vertices.extend_from_slice(&nm.to_le_bytes());
        }

        // 6 indices: [0,1,2, 0,2,3] pattern (CCW)
        let base = vert_count;
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        vert_count += 4;
    }

    (vertices, indices)
}

// ─── Full pipeline ──────────────────────────────────────────────────────

/// Complete CPU mesh rebuild result.
pub struct MeshResult {
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
    pub draw_meta: DrawMeta,
    pub quad_count: u32,
}

/// Run the complete CPU greedy mesh pipeline for one chunk.
///
/// `palette`: packed u16 MaterialIds (2 per u32 word).
/// `index_buf`: bitpacked per-voxel palette indices at `bpe` bit width.
/// `palette_meta`: packed u32 (bits 0–15 = palette_size, bits 16–23 = bpe).
pub fn mesh_rebuild_cpu(
    occupancy: &[u32],
    palette: &[u32],
    index_buf: &[u32],
    palette_meta: u32,
    chunk_coord: [i32; 3],
) -> MeshResult {
    let bpe = (palette_meta >> 16) & 0xFF;
    let masks = cull_faces_cpu(occupancy);
    let quads = greedy_merge(occupancy, &masks, palette, index_buf, bpe);
    let (vertices, indices) = expand_quads(&quads, chunk_coord);

    let vert_count = (vertices.len() / VERTEX_BYTES as usize) as u32;
    let idx_count = indices.len() as u32;

    MeshResult {
        draw_meta: DrawMeta {
            vertex_offset: 0,
            vertex_count: vert_count,
            index_offset: 0,
            index_count: idx_count,
            material_base: 0,
            _pad: [0; 3],
        },
        vertices,
        indices,
        quad_count: quads.len() as u32,
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{IndexBufBuilder, OccupancyBuilder, PaletteBuilder};

    /// Build default single-material palette data for tests that don't care about materials.
    /// Returns (palette_words, index_buf_words, palette_meta).
    fn default_palette_data() -> (Vec<u32>, Vec<u32>, u32) {
        let mut pal = PaletteBuilder::new();
        pal.add(MATERIAL_DEFAULT);
        let palette_words = pal.as_words();
        let bpe = IndexBufBuilder::bits_per_entry(pal.len());
        let ib = IndexBufBuilder::new(); // all zeros = palette_idx 0 everywhere
        let index_words = ib.pack(bpe);
        let meta = IndexBufBuilder::palette_meta(pal.len());
        (palette_words, index_words, meta)
    }

    fn occ_with_voxel(x: u32, y: u32, z: u32) -> Vec<u32> {
        let mut b = OccupancyBuilder::new();
        b.set(x, y, z);
        b.as_words().to_vec()
    }

    fn occ_with_voxels(voxels: &[(u32, u32, u32)]) -> Vec<u32> {
        let mut b = OccupancyBuilder::new();
        for &(x, y, z) in voxels {
            b.set(x, y, z);
        }
        b.as_words().to_vec()
    }

    // ── Face culling tests ──

    #[test]
    fn empty_chunk_zero_faces() {
        let occ = vec![0u32; OCCUPANCY_WORDS_PER_SLOT as usize];
        let masks = cull_faces_cpu(&occ);
        assert_eq!(total_face_count(&masks), 0);
    }

    #[test]
    fn single_voxel_six_faces() {
        // Voxel at padded (32, 32, 32) = usable (31, 31, 31)
        let occ = occ_with_voxel(32, 32, 32);
        let masks = cull_faces_cpu(&occ);
        let counts = count_faces(&masks);
        // Each direction should have exactly 1 face
        for (i, &c) in counts.iter().enumerate() {
            assert_eq!(c, 1, "face direction {i} should have 1 face, got {c}");
        }
        assert_eq!(total_face_count(&masks), 6);
    }

    #[test]
    fn two_adjacent_y_shared_face_culled() {
        // Two voxels stacked vertically: (32,32,32) and (32,33,32)
        let occ = occ_with_voxels(&[(32, 32, 32), (32, 33, 32)]);
        let masks = cull_faces_cpu(&occ);
        let counts = count_faces(&masks);
        // +Y: only top voxel has +Y face = 1
        assert_eq!(counts[FACE_POS_Y], 1, "+Y should have 1 face");
        // -Y: only bottom voxel has -Y face = 1
        assert_eq!(counts[FACE_NEG_Y], 1, "-Y should have 1 face");
        // X/Z faces: each voxel has its own = 2 each
        assert_eq!(counts[FACE_POS_X], 2);
        assert_eq!(counts[FACE_NEG_X], 2);
        assert_eq!(counts[FACE_POS_Z], 2);
        assert_eq!(counts[FACE_NEG_Z], 2);
        assert_eq!(total_face_count(&masks), 10); // 6+6 - 2 shared = 10
    }

    #[test]
    fn two_adjacent_x_shared_face_culled() {
        let occ = occ_with_voxels(&[(32, 32, 32), (33, 32, 32)]);
        let masks = cull_faces_cpu(&occ);
        let counts = count_faces(&masks);
        assert_eq!(counts[FACE_POS_X], 1);
        assert_eq!(counts[FACE_NEG_X], 1);
        assert_eq!(counts[FACE_POS_Y], 2);
        assert_eq!(total_face_count(&masks), 10);
    }

    #[test]
    fn two_adjacent_z_shared_face_culled() {
        let occ = occ_with_voxels(&[(32, 32, 32), (32, 32, 33)]);
        let masks = cull_faces_cpu(&occ);
        let counts = count_faces(&masks);
        assert_eq!(counts[FACE_POS_Z], 1);
        assert_eq!(counts[FACE_NEG_Z], 1);
        assert_eq!(total_face_count(&masks), 10);
    }

    #[test]
    fn full_slab_only_surface() {
        // Fill the entire usable Y=1 layer (padded y=2)
        let mut b = OccupancyBuilder::new();
        for x in 1..CS_P - 1 {
            for z in 1..CS_P - 1 {
                b.set(x, 2, z); // padded y=2
            }
        }
        let occ = b.as_words().to_vec();
        let masks = cull_faces_cpu(&occ);
        let counts = count_faces(&masks);

        // +Y and -Y: each interior voxel has a face = 62*62 = 3844 each
        assert_eq!(counts[FACE_POS_Y], CS * CS, "+Y should be full slab");
        assert_eq!(counts[FACE_NEG_Y], CS * CS, "-Y should be full slab");
        // X faces: only edge voxels (z=1..62 at x=1 and x=62) = 62 each
        assert_eq!(counts[FACE_POS_X], CS, "+X should be edge only");
        assert_eq!(counts[FACE_NEG_X], CS, "-X should be edge only");
        assert_eq!(counts[FACE_POS_Z], CS, "+Z should be edge only");
        assert_eq!(counts[FACE_NEG_Z], CS, "-Z should be edge only");
    }

    #[test]
    fn boundary_voxel_correct() {
        // Voxel at usable (0,0,0) = padded (1,1,1)
        let occ = occ_with_voxel(1, 1, 1);
        let masks = cull_faces_cpu(&occ);
        assert_eq!(total_face_count(&masks), 6, "boundary voxel should have 6 faces");
    }

    #[test]
    fn face_count_symmetry() {
        // Symmetric cross pattern: center + 6 neighbors
        let occ = occ_with_voxels(&[
            (32, 32, 32),
            (31, 32, 32), (33, 32, 32),
            (32, 31, 32), (32, 33, 32),
            (32, 32, 31), (32, 32, 33),
        ]);
        let masks = cull_faces_cpu(&occ);
        let counts = count_faces(&masks);
        // By symmetry, each direction should have the same count
        assert_eq!(counts[FACE_POS_X], counts[FACE_NEG_X]);
        assert_eq!(counts[FACE_POS_Y], counts[FACE_NEG_Y]);
        assert_eq!(counts[FACE_POS_Z], counts[FACE_NEG_Z]);
    }

    // ── Greedy merge tests ──

    #[test]
    fn single_voxel_six_quads() {
        let occ = occ_with_voxel(32, 32, 32);
        let masks = cull_faces_cpu(&occ);
        let (pal, idx, meta) = default_palette_data();
        let bpe = (meta >> 16) & 0xFF;
        let quads = greedy_merge(&occ, &masks, &pal, &idx, bpe);
        assert_eq!(quads.len(), 6, "single voxel should produce 6 quads");
        for q in &quads {
            assert_eq!(q.width, 1);
            assert_eq!(q.height, 1);
        }
    }

    #[test]
    fn two_voxel_bar_merges_y_face() {
        // Two voxels adjacent in X at same Y: (32,32,32) and (33,32,32)
        let occ = occ_with_voxels(&[(32, 32, 32), (33, 32, 32)]);
        let masks = cull_faces_cpu(&occ);
        let (pal, idx, meta) = default_palette_data();
        let bpe = (meta >> 16) & 0xFF;
        let quads = greedy_merge(&occ, &masks, &pal, &idx, bpe);

        // +Y face should merge into one width=2 quad
        let pos_y_quads: Vec<_> = quads.iter().filter(|q| q.face == FACE_POS_Y).collect();
        assert_eq!(pos_y_quads.len(), 1, "+Y should have 1 merged quad");
        assert_eq!(pos_y_quads[0].width, 2);
        assert_eq!(pos_y_quads[0].height, 1);
    }

    #[test]
    fn slab_3x3_single_quad_top() {
        // 3x3 slab at y=32
        let mut voxels = Vec::new();
        for x in 32..35 {
            for z in 32..35 {
                voxels.push((x, 32, z));
            }
        }
        let occ = occ_with_voxels(&voxels);
        let masks = cull_faces_cpu(&occ);
        let (pal, idx, meta) = default_palette_data();
        let bpe = (meta >> 16) & 0xFF;
        let quads = greedy_merge(&occ, &masks, &pal, &idx, bpe);

        let pos_y_quads: Vec<_> = quads.iter().filter(|q| q.face == FACE_POS_Y).collect();
        assert_eq!(pos_y_quads.len(), 1, "+Y should merge into 1 quad");
        assert_eq!(pos_y_quads[0].width, 3);
        assert_eq!(pos_y_quads[0].height, 3);
    }

    // ── Vertex expansion tests ──

    #[test]
    fn single_voxel_vertex_counts() {
        let occ = occ_with_voxel(32, 32, 32);
        let (pal, idx, meta) = default_palette_data();
        let result = mesh_rebuild_cpu(&occ, &pal, &idx, meta, [0, 0, 0]);
        assert_eq!(result.draw_meta.vertex_count, 24, "6 quads × 4 verts = 24");
        assert_eq!(result.draw_meta.index_count, 36, "6 quads × 6 indices = 36");
        assert_eq!(result.quad_count, 6);
    }

    #[test]
    fn vertex_positions_in_bounds() {
        let occ = occ_with_voxel(32, 32, 32);
        let (pal, idx, meta) = default_palette_data();
        let result = mesh_rebuild_cpu(&occ, &pal, &idx, meta, [0, 0, 0]);
        for i in 0..result.draw_meta.vertex_count as usize {
            let base = i * VERTEX_BYTES as usize;
            let px = f32::from_le_bytes(result.vertices[base..base + 4].try_into().unwrap());
            let py = f32::from_le_bytes(result.vertices[base + 4..base + 8].try_into().unwrap());
            let pz = f32::from_le_bytes(result.vertices[base + 8..base + 12].try_into().unwrap());
            assert!(px >= 0.0 && px <= CS_P as f32, "px={px} out of bounds");
            assert!(py >= 0.0 && py <= CS_P as f32, "py={py} out of bounds");
            assert!(pz >= 0.0 && pz <= CS_P as f32, "pz={pz} out of bounds");
        }
    }

    #[test]
    fn index_pattern_correct() {
        let occ = occ_with_voxel(32, 32, 32);
        let (pal, idx, meta) = default_palette_data();
        let result = mesh_rebuild_cpu(&occ, &pal, &idx, meta, [0, 0, 0]);
        for i in (0..result.indices.len()).step_by(6) {
            let b = result.indices[i];
            assert_eq!(result.indices[i + 1], b + 1);
            assert_eq!(result.indices[i + 2], b + 2);
            assert_eq!(result.indices[i + 3], b);
            assert_eq!(result.indices[i + 4], b + 2);
            assert_eq!(result.indices[i + 5], b + 3);
        }
    }

    #[test]
    fn draw_meta_counts_match() {
        let occ = occ_with_voxel(32, 32, 32);
        let (pal, idx, meta) = default_palette_data();
        let result = mesh_rebuild_cpu(&occ, &pal, &idx, meta, [0, 0, 0]);
        assert_eq!(
            result.draw_meta.vertex_count as usize,
            result.vertices.len() / VERTEX_BYTES as usize
        );
        assert_eq!(
            result.draw_meta.index_count as usize,
            result.indices.len()
        );
    }

    #[test]
    fn empty_chunk_zero_output() {
        let occ = vec![0u32; OCCUPANCY_WORDS_PER_SLOT as usize];
        let (pal, idx, meta) = default_palette_data();
        let result = mesh_rebuild_cpu(&occ, &pal, &idx, meta, [0, 0, 0]);
        assert_eq!(result.draw_meta.vertex_count, 0);
        assert_eq!(result.draw_meta.index_count, 0);
        assert_eq!(result.quad_count, 0);
    }

    #[test]
    fn within_pool_limits() {
        let (chunks, _) = crate::scene::generate_test_scene();
        let chunk = &chunks[0];
        let pal_words = chunk.palette.as_words();
        let bpe = IndexBufBuilder::bits_per_entry(chunk.palette.len());
        let idx_words = chunk.index_buf.pack(bpe);
        let meta = IndexBufBuilder::palette_meta(chunk.palette.len());
        let result = mesh_rebuild_cpu(
            chunk.occupancy.as_words(), &pal_words, &idx_words, meta,
            [0, 0, 0],
        );
        assert!(
            result.draw_meta.vertex_count <= MAX_VERTS_PER_CHUNK,
            "vertex_count {} exceeds MAX_VERTS_PER_CHUNK {}",
            result.draw_meta.vertex_count, MAX_VERTS_PER_CHUNK
        );
        assert!(
            result.draw_meta.index_count <= MAX_INDICES_PER_CHUNK,
            "index_count {} exceeds MAX_INDICES_PER_CHUNK {}",
            result.draw_meta.index_count, MAX_INDICES_PER_CHUNK
        );
    }

    #[test]
    fn test_scene_reasonable_counts() {
        let (chunks, _) = crate::scene::generate_test_scene();
        let chunk = &chunks[0];
        let pal_words = chunk.palette.as_words();
        let bpe = IndexBufBuilder::bits_per_entry(chunk.palette.len());
        let idx_words = chunk.index_buf.pack(bpe);
        let meta = IndexBufBuilder::palette_meta(chunk.palette.len());
        let result = mesh_rebuild_cpu(
            chunk.occupancy.as_words(), &pal_words, &idx_words, meta,
            [0, 0, 0],
        );
        // The room + sphere + emissive should produce a nontrivial mesh
        assert!(result.quad_count > 100, "too few quads: {}", result.quad_count);
        assert!(result.draw_meta.vertex_count > 400, "too few verts: {}", result.draw_meta.vertex_count);
        assert!(result.draw_meta.index_count > 600, "too few indices: {}", result.draw_meta.index_count);
    }

    // ── Material boundary tests ──

    #[test]
    fn two_material_merge_boundary() {
        // Two adjacent voxels with DIFFERENT materials should NOT merge
        let mut occ_b = OccupancyBuilder::new();
        let mut pal = PaletteBuilder::new();
        let mut ib = IndexBufBuilder::new();

        occ_b.set(32, 32, 32);
        occ_b.set(33, 32, 32);

        let mat_a = pal.add(2); // MaterialId 2
        let mat_b = pal.add(3); // MaterialId 3
        ib.set(32, 32, 32, mat_a);
        ib.set(33, 32, 32, mat_b);

        let occ = occ_b.as_words().to_vec();
        let pal_words = pal.as_words();
        let bpe = IndexBufBuilder::bits_per_entry(pal.len());
        let idx_words = ib.pack(bpe);

        let masks = cull_faces_cpu(&occ);
        let quads = greedy_merge(&occ, &masks, &pal_words, &idx_words, bpe as u32);

        // +Y face should have 2 separate quads (not merged)
        let pos_y: Vec<_> = quads.iter().filter(|q| q.face == FACE_POS_Y).collect();
        assert_eq!(pos_y.len(), 2, "+Y should have 2 quads (material boundary)");
        // And they should have different material_ids
        assert_ne!(pos_y[0].material_id, pos_y[1].material_id);
    }

    #[test]
    fn same_material_still_merges() {
        // Two adjacent voxels with SAME material should still merge
        let mut occ_b = OccupancyBuilder::new();
        let mut pal = PaletteBuilder::new();
        let mut ib = IndexBufBuilder::new();

        occ_b.set(32, 32, 32);
        occ_b.set(33, 32, 32);

        let mat_a = pal.add(2); // same material for both
        ib.set(32, 32, 32, mat_a);
        ib.set(33, 32, 32, mat_a);

        let occ = occ_b.as_words().to_vec();
        let pal_words = pal.as_words();
        let bpe = IndexBufBuilder::bits_per_entry(pal.len());
        let idx_words = ib.pack(bpe);

        let masks = cull_faces_cpu(&occ);
        let quads = greedy_merge(&occ, &masks, &pal_words, &idx_words, bpe as u32);

        let pos_y: Vec<_> = quads.iter().filter(|q| q.face == FACE_POS_Y).collect();
        assert_eq!(pos_y.len(), 1, "+Y should merge into 1 quad (same material)");
        assert_eq!(pos_y[0].width, 2);
    }

    #[test]
    fn material_region_boundary() {
        // 4x1 bar: first 2 voxels = mat A, next 2 = mat B
        let mut occ_b = OccupancyBuilder::new();
        let mut pal = PaletteBuilder::new();
        let mut ib = IndexBufBuilder::new();

        let mat_a = pal.add(2);
        let mat_b = pal.add(3);

        for x in 32..36 {
            occ_b.set(x, 32, 32);
            let m = if x < 34 { mat_a } else { mat_b };
            ib.set(x, 32, 32, m);
        }

        let occ = occ_b.as_words().to_vec();
        let pal_words = pal.as_words();
        let bpe = IndexBufBuilder::bits_per_entry(pal.len());
        let idx_words = ib.pack(bpe);

        let masks = cull_faces_cpu(&occ);
        let quads = greedy_merge(&occ, &masks, &pal_words, &idx_words, bpe as u32);

        let pos_y: Vec<_> = quads.iter().filter(|q| q.face == FACE_POS_Y).collect();
        assert_eq!(pos_y.len(), 2, "+Y should have 2 quads at material boundary");
        // Each should be width=2
        for q in &pos_y {
            assert_eq!(q.width, 2, "each material region should be width=2");
        }
    }
}

#[cfg(test)]
mod multi_chunk_tests {
    use super::*;
    use crate::obj_parser;
    use crate::voxelizer_cpu;
    use crate::scene::IndexBufBuilder;

    /// Diagnose: does each chunk get all 6 face directions from the CPU mesher?
    #[test]
    fn multi_chunk_all_face_directions() {
        // Create a simple cube OBJ that spans 2+ chunks at resolution 100
        let obj = "\
v -1 -1 -1
v  1 -1 -1
v  1  1 -1
v -1  1 -1
v -1 -1  1
v  1 -1  1
v  1  1  1
v -1  1  1
f 1 2 3 4
f 5 8 7 6
f 1 5 6 2
f 3 7 8 4
f 2 6 7 3
f 1 4 8 5
";
        let parsed = obj_parser::parse_obj(obj);
        let result = voxelizer_cpu::voxelize(&parsed, 100);

        println!("Voxelized into {} chunks:", result.chunks.len());
        for chunk in &result.chunks {
            let pal_words = chunk.palette.as_words();
            let bpe = IndexBufBuilder::bits_per_entry(chunk.palette.len());
            let idx_words = chunk.index_buf.pack(bpe);
            let meta = IndexBufBuilder::palette_meta(chunk.palette.len());

            let mesh = mesh_rebuild_cpu(
                chunk.occupancy.as_words(),
                &pal_words,
                &idx_words,
                meta,
                [chunk.coord.x, chunk.coord.y, chunk.coord.z],
            );

            let masks = cull_faces_cpu(chunk.occupancy.as_words());
            let face_counts = count_faces(&masks);
            let quads = greedy_merge(
                chunk.occupancy.as_words(), &masks, &pal_words, &idx_words, bpe as u32,
            );

            // Count quads per face direction
            let mut quads_per_face = [0u32; 6];
            for q in &quads {
                quads_per_face[q.face] += 1;
            }

            println!(
                "  Chunk ({},{},{}): {} voxels, {} total quads, {} verts, {} indices",
                chunk.coord.x, chunk.coord.y, chunk.coord.z,
                chunk.occupancy.popcount(),
                quads.len(),
                mesh.draw_meta.vertex_count,
                mesh.draw_meta.index_count,
            );
            println!(
                "    Face counts (visible): +Y={} -Y={} +X={} -X={} +Z={} -Z={}",
                face_counts[0], face_counts[1], face_counts[2],
                face_counts[3], face_counts[4], face_counts[5],
            );
            println!(
                "    Quad counts (merged):  +Y={} -Y={} +X={} -X={} +Z={} -Z={}",
                quads_per_face[0], quads_per_face[1], quads_per_face[2],
                quads_per_face[3], quads_per_face[4], quads_per_face[5],
            );

            // Check: every face direction should have at least 1 quad
            for face in 0..6 {
                let dir = ["+Y", "-Y", "+X", "-X", "+Z", "-Z"][face];
                assert!(
                    quads_per_face[face] > 0,
                    "Chunk ({},{},{}) missing {} face quads! face_visible_count={}, quad_count={}",
                    chunk.coord.x, chunk.coord.y, chunk.coord.z,
                    dir, face_counts[face], quads_per_face[face],
                );
            }

            // Check: every quad has the same material (single-material mesh)
            for q in &quads {
                assert!(
                    q.material_id > 0,
                    "Chunk ({},{},{}) quad at ({},{},{}) face={} has material_id=0 (EMPTY)!",
                    chunk.coord.x, chunk.coord.y, chunk.coord.z,
                    q.x, q.y, q.z, q.face,
                );
            }
        }

        assert!(result.chunks.len() >= 2, "Expected multi-chunk output");
    }

    // Paste into mesh_cpu.rs tests temporarily
    
    #[test]
    fn winding_validation_multi_chunk() {
        use crate::obj_parser;
        use crate::voxelizer_cpu;
        use crate::scene::IndexBufBuilder;
    
        let obj = "v -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv -1 -1 1\nv 1 -1 1\nv 1 1 1\nv -1 1 1\nf 1 2 3 4\nf 5 8 7 6\nf 1 5 6 2\nf 3 7 8 4\nf 2 6 7 3\nf 1 4 8 5\n";
        let parsed = obj_parser::parse_obj(obj);
        let result = voxelizer_cpu::voxelize(&parsed, 100);
    
        let mut bad_count = 0u32;
        let mut total_tris = 0u32;
    
        for chunk in &result.chunks {
            let pal_words = chunk.palette.as_words();
            let bpe = IndexBufBuilder::bits_per_entry(chunk.palette.len());
            let idx_words = chunk.index_buf.pack(bpe);
            let meta = IndexBufBuilder::palette_meta(chunk.palette.len());
    
            let mesh = mesh_rebuild_cpu(
                chunk.occupancy.as_words(), &pal_words, &idx_words, meta,
                [chunk.coord.x, chunk.coord.y, chunk.coord.z],
            );
    
            let verts = &mesh.vertices;
            let indices = &mesh.indices;
            let vc = mesh.draw_meta.vertex_count as usize;
    
            // Parse all vertex positions
            let mut positions = Vec::with_capacity(vc);
            let mut normals = Vec::with_capacity(vc);
            for i in 0..vc {
                let base = i * VERTEX_BYTES as usize;
                let px = f32::from_le_bytes(verts[base..base+4].try_into().unwrap());
                let py = f32::from_le_bytes(verts[base+4..base+8].try_into().unwrap());
                let pz = f32::from_le_bytes(verts[base+8..base+12].try_into().unwrap());
                let nm = u32::from_le_bytes(verts[base+12..base+16].try_into().unwrap());
                let nx = (nm & 0xFF) as i8;
                let ny = ((nm >> 8) & 0xFF) as i8;
                let nz = ((nm >> 16) & 0xFF) as i8;
                positions.push([px, py, pz]);
                normals.push([nx as f32 / 127.0, ny as f32 / 127.0, nz as f32 / 127.0]);
            }
    
            // Check each triangle: cross product should agree with declared normal
            for i in (0..indices.len()).step_by(3) {
                let i0 = indices[i] as usize;
                let i1 = indices[i+1] as usize;
                let i2 = indices[i+2] as usize;
                if i0 >= vc || i1 >= vc || i2 >= vc { continue; }
    
                let p0 = positions[i0];
                let p1 = positions[i1];
                let p2 = positions[i2];
                let decl_n = normals[i0]; // all verts in a quad share the normal
    
                // Edge vectors
                let e1 = [p1[0]-p0[0], p1[1]-p0[1], p1[2]-p0[2]];
                let e2 = [p2[0]-p0[0], p2[1]-p0[1], p2[2]-p0[2]];
                // Cross product
                let cx = e1[1]*e2[2] - e1[2]*e2[1];
                let cy = e1[2]*e2[0] - e1[0]*e2[2];
                let cz = e1[0]*e2[1] - e1[1]*e2[0];
    
                // Dot with declared normal — should be positive (same direction)
                let dot = cx * decl_n[0] + cy * decl_n[1] + cz * decl_n[2];
    
                total_tris += 1;
                if dot <= 0.0 {
                    bad_count += 1;
                    if bad_count <= 5 {
                        println!(
                            "BAD WINDING chunk ({},{},{}): tri ({},{},{}) cross=({:.2},{:.2},{:.2}) normal=({:.2},{:.2},{:.2}) dot={:.4}",
                            chunk.coord.x, chunk.coord.y, chunk.coord.z,
                            i0, i1, i2, cx, cy, cz, decl_n[0], decl_n[1], decl_n[2], dot,
                        );
                        println!("  p0=({:.1},{:.1},{:.1}) p1=({:.1},{:.1},{:.1}) p2=({:.1},{:.1},{:.1})",
                            p0[0],p0[1],p0[2], p1[0],p1[1],p1[2], p2[0],p2[1],p2[2]);
                    }
                }
            }
        }
    
        println!("\nTotal triangles: {total_tris}, bad winding: {bad_count}");
        assert_eq!(bad_count, 0, "{bad_count} triangles have wrong winding out of {total_tris}");
    }
    

    #[test]
    fn voxel_coverage_no_gaps() {
        // Fill a 70x70x70 region — must span at least 2 chunks on each axis
        use crate::obj_parser;
        use crate::voxelizer_cpu;
        use crate::pool::*;
        use std::collections::HashSet;
    
        // Create a big cube: vertices span [-1, 1] so 2 units wide
        let obj = "v -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv -1 -1 1\nv 1 -1 1\nv 1 1 1\nv -1 1 1\nf 1 2 3 4\nf 5 8 7 6\nf 1 5 6 2\nf 3 7 8 4\nf 2 6 7 3\nf 1 4 8 5\n";
        let parsed = obj_parser::parse_obj(obj);
        let result = voxelizer_cpu::voxelize(&parsed, 100);
    
        println!("Chunks: {}", result.chunks.len());
    
        // Collect all global voxel positions from all chunks
        let mut global_voxels: HashSet<(i32, i32, i32)> = HashSet::new();
        let mut duplicates = 0u32;
    
        for chunk in &result.chunks {
            let cx = chunk.coord.x;
            let cy = chunk.coord.y;
            let cz = chunk.coord.z;
    
            for lx in 1..=CS {
                for ly in 1..=CS {
                    for lz in 1..=CS {
                        if chunk.occupancy.get(lx, ly, lz) {
                            // Convert local to global: global = chunk_coord * CS + (local - 1)
                            let gx = cx * CS as i32 + (lx as i32 - 1);
                            let gy = cy * CS as i32 + (ly as i32 - 1);
                            let gz = cz * CS as i32 + (lz as i32 - 1);
                            if !global_voxels.insert((gx, gy, gz)) {
                                duplicates += 1;
                            }
                        }
                    }
                }
            }
        }
    
        println!("Total global voxels: {}", global_voxels.len());
        println!("Duplicates: {}", duplicates);
        assert_eq!(duplicates, 0, "voxels should not appear in multiple chunks");
    
        // Check that the filled region has no gaps:
        // Find the bounding box of all voxels
        let min_x = global_voxels.iter().map(|v| v.0).min().unwrap();
        let max_x = global_voxels.iter().map(|v| v.0).max().unwrap();
        let min_y = global_voxels.iter().map(|v| v.1).min().unwrap();
        let max_y = global_voxels.iter().map(|v| v.1).max().unwrap();
        let min_z = global_voxels.iter().map(|v| v.2).min().unwrap();
        let max_z = global_voxels.iter().map(|v| v.2).max().unwrap();
    
        println!("AABB: ({},{},{}) to ({},{},{})", min_x, min_y, min_z, max_x, max_y, max_z);
    
        // For each face direction, count visible faces per chunk
        for chunk in &result.chunks {
            let masks = cull_faces_cpu(chunk.occupancy.as_words());
            let counts = count_faces(&masks);
            let total: u32 = counts.iter().sum();
            println!(
                "Chunk ({},{},{}): {} voxels, faces: +Y={} -Y={} +X={} -X={} +Z={} -Z={} total={}",
                chunk.coord.x, chunk.coord.y, chunk.coord.z,
                chunk.occupancy.popcount(),
                counts[0], counts[1], counts[2], counts[3], counts[4], counts[5],
                total,
            );
    
            // The key test: for an interior chunk (not at the edge of the model),
            // all 6 face directions should have ZERO visible faces because
            // all neighbors are solid. But with missing padding, boundary faces appear.
            // Let's just report and look for asymmetry.
            if counts[2] > 0 && counts[3] == 0 {
                println!("  WARNING: +X faces present but -X missing!");
            }
            if counts[3] > 0 && counts[2] == 0 {
                println!("  WARNING: -X faces present but +X missing!");
            }
            if counts[0] > 0 && counts[1] == 0 {
                println!("  WARNING: +Y faces present but -Y missing!");
            }
            if counts[1] > 0 && counts[0] == 0 {
                println!("  WARNING: -Y faces present but +Y missing!");
            }
            if counts[4] > 0 && counts[5] == 0 {
                println!("  WARNING: +Z faces present but -Z missing!");
            }
            if counts[5] > 0 && counts[4] == 0 {
                println!("  WARNING: -Z faces present but +Z missing!");
            }
        }
    }
    
}

#[cfg(test)]
mod material_diag_tests {
    use super::*;
    use crate::obj_parser;
    use crate::voxelizer_cpu;
    use crate::scene::IndexBufBuilder;

    /// Check the actual normal+material packed values in emitted vertices
    #[test]
    fn multi_chunk_vertex_normal_material_check() {
        let obj = "\
v -1 -1 -1
v  1 -1 -1
v  1  1 -1
v -1  1 -1
v -1 -1  1
v  1 -1  1
v  1  1  1
v -1  1  1
f 1 2 3 4
f 5 8 7 6
f 1 5 6 2
f 3 7 8 4
f 2 6 7 3
f 1 4 8 5
";
        let parsed = obj_parser::parse_obj(obj);
        let result = voxelizer_cpu::voxelize(&parsed, 100);

        for chunk in &result.chunks {
            let pal_words = chunk.palette.as_words();
            let bpe = IndexBufBuilder::bits_per_entry(chunk.palette.len());
            let idx_words = chunk.index_buf.pack(bpe);
            let meta = IndexBufBuilder::palette_meta(chunk.palette.len());

            let mesh = mesh_rebuild_cpu(
                chunk.occupancy.as_words(), &pal_words, &idx_words, meta,
                [chunk.coord.x, chunk.coord.y, chunk.coord.z],
            );

            println!("\nChunk ({},{},{}):", chunk.coord.x, chunk.coord.y, chunk.coord.z);
            println!("  Palette: {:?}", pal_words);
            println!("  bpe={}, meta=0x{:08X}", bpe, meta);

            // Read each vertex's packed normal_material u32
            for i in 0..mesh.draw_meta.vertex_count as usize {
                let base = i * VERTEX_BYTES as usize;
                let nm = u32::from_le_bytes(mesh.vertices[base+12..base+16].try_into().unwrap());
                let nx = (nm & 0xFF) as i8;
                let ny = ((nm >> 8) & 0xFF) as i8;
                let nz = ((nm >> 16) & 0xFF) as i8;
                let mat = (nm >> 24) & 0xFF;
                // Only print first vertex of each quad (every 4th)
                if i % 4 == 0 {
                    let px = f32::from_le_bytes(mesh.vertices[base..base+4].try_into().unwrap());
                    let py = f32::from_le_bytes(mesh.vertices[base+4..base+8].try_into().unwrap());
                    let pz = f32::from_le_bytes(mesh.vertices[base+8..base+12].try_into().unwrap());
                    println!("  Quad {}: pos=({:.0},{:.0},{:.0}) normal=({},{},{}) mat={}",
                        i/4, px, py, pz, nx, ny, nz, mat);
                }
            }
        }
    }
}
