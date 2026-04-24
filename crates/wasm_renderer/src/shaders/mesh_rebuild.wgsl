// R-1 Pass 3: Mesh Write Compute Shader
//
// Writes vertices and indices at offsets computed by the prefix sum (Pass 2).
// Same face-cull + greedy-merge + material-aware algorithm as mesh_count.wgsl.
//
// Dispatch: (slot_count, 6, 1), @workgroup_size(64, 1, 1).
//
// See: docs/Resident Representation/variable-mesh-pool.md

// ─── Constants ──────────────────────────────────────────────────────────

const CS_P: u32 = 64u;
const CS: u32 = 62u;
const COLUMNS_PER_CHUNK: u32 = 4096u;
const WORDS_PER_SLOT: u32 = 8192u;
const VERTEX_STRIDE: u32 = 4u;  // 4 u32 per vertex (vec3f + u32)

// Face directions
const FACE_POS_Y: u32 = 0u;
const FACE_NEG_Y: u32 = 1u;
const FACE_POS_X: u32 = 2u;
const FACE_NEG_X: u32 = 3u;
const FACE_POS_Z: u32 = 4u;
const FACE_NEG_Z: u32 = 5u;

// Usable mask: bits [0..61] set (62 bits)
const USABLE_MASK_LO: u32 = 0xFFFFFFFFu;  // bits 0..31
const USABLE_MASK_HI: u32 = 0x3FFFFFFFu;  // bits 0..29 (total 62 bits)

// ─── Bindings ───────────────────────────────────────────────────────────

@group(0) @binding(0) var<storage, read>       occupancy:          array<u32>;
@group(0) @binding(1) var<storage, read>       palette:            array<u32>;
@group(0) @binding(2) var<storage, read>       coord:              array<vec4i>;
@group(0) @binding(3) var<storage, read_write> vertex_pool:        array<u32>;
@group(0) @binding(4) var<storage, read_write> index_pool:         array<u32>;
@group(0) @binding(5) var<storage, read_write> mesh_offset_table:  array<atomic<u32>>;
@group(0) @binding(6) var<storage, read>       index_buf_pool:     array<u32>;
@group(0) @binding(7) var<storage, read>       palette_meta:       array<u32>;
@group(0) @binding(8) var<uniform>             scene_params:       vec4f; // xyz=grid_origin, w=voxel_size

// ─── Helpers ────────────────────────────────────────────────────────────

// Read a u64 column as two u32 from the occupancy buffer.
fn read_col(slot_offset: u32, x: u32, z: u32) -> vec2u {
    let col_idx = x * CS_P + z;
    let base = slot_offset + col_idx * 2u;
    return vec2u(occupancy[base], occupancy[base + 1u]);
}

// Shift a u64 (stored as vec2u lo,hi) right by 1.
fn shr1(v: vec2u) -> vec2u {
    let lo = (v.x >> 1u) | (v.y << 31u);
    let hi = v.y >> 1u;
    return vec2u(lo, hi);
}

// Shift a u64 (stored as vec2u lo,hi) left by 1.
fn shl1(v: vec2u) -> vec2u {
    let lo = v.x << 1u;
    let hi = (v.y << 1u) | (v.x >> 31u);
    return vec2u(lo, hi);
}

// Bitwise NOT of a vec2u.
fn not2(v: vec2u) -> vec2u {
    return vec2u(~v.x, ~v.y);
}

// Bitwise AND of two vec2u.
fn and2(a: vec2u, b: vec2u) -> vec2u {
    return vec2u(a.x & b.x, a.y & b.y);
}

// Apply usable mask: shift right 1, then mask to 62 bits.
fn to_usable(v: vec2u) -> vec2u {
    let shifted = shr1(v);
    return vec2u(shifted.x & USABLE_MASK_LO, shifted.y & USABLE_MASK_HI);
}

// Test if bit y is set in a u64 stored as vec2u.
fn bit_set(v: vec2u, y: u32) -> bool {
    if y < 32u {
        return (v.x >> y & 1u) != 0u;
    } else {
        return (v.y >> (y - 32u) & 1u) != 0u;
    }
}

// Popcount of a vec2u (u64).
fn popcount2(v: vec2u) -> u32 {
    return countOneBits(v.x) + countOneBits(v.y);
}

// Compute face mask for one column and one direction.
fn cull_column(col: vec2u, neighbor: vec2u, face: u32) -> vec2u {
    switch face {
        case 0u: { // +Y: col & !(col >> 1)
            return to_usable(and2(col, not2(shr1(col))));
        }
        case 1u: { // -Y: col & !(col << 1)
            return to_usable(and2(col, not2(shl1(col))));
        }
        default: { // ±X, ±Z: col & !neighbor
            return to_usable(and2(col, not2(neighbor)));
        }
    }
}

// Get the neighbor column for a given face direction.
fn get_neighbor(slot_offset: u32, x: u32, z: u32, face: u32) -> vec2u {
    switch face {
        case 2u: { return read_col(slot_offset, x + 1u, z); }     // +X
        case 3u: { return read_col(slot_offset, x - 1u, z); }     // -X
        case 4u: { return read_col(slot_offset, x, z + 1u); }     // +Z
        case 5u: { return read_col(slot_offset, x, z - 1u); }     // -Z
        default: { return vec2u(0u, 0u); }                         // Y faces don't use neighbor
    }
}

// ─── Material lookup ────────────────────────────────────────────────────
//
// Resolves global MaterialId for a voxel from bitpacked index_buf → palette.
// See: docs/Resident Representation/material-aware-merge.md

const PALETTE_WORDS_PER_SLOT: u32 = 128u;      // 256 entries / 2 per u32

// Resolve global MaterialId for a voxel at padded coords (px, py, pz).
// bpe (bits_per_entry) must be 1, 2, 4, or 8. Entries never span u32 words (IDX-1).
fn read_material_id(slot: u32, px: u32, py: u32, pz: u32, bpe: u32) -> u32 {
    // Decode palette index from bitpacked index buffer (variable allocation)
    let voxel_index = px * 4096u + py * 64u + pz;
    let bit_offset = voxel_index * bpe;
    let word_index = bit_offset >> 5u;
    let bit_within = bit_offset & 31u;
    let mask = (1u << bpe) - 1u;
    let slot_base = palette_meta[slot * 2u + 1u]; // index_buf_word_offset from palette_meta
    let palette_idx = (index_buf_pool[slot_base + word_index] >> bit_within) & mask;

    // Resolve global MaterialId from palette
    let pal_base = slot * PALETTE_WORDS_PER_SLOT;
    let pal_word = palette[pal_base + (palette_idx >> 1u)];
    let shift = (palette_idx & 1u) * 16u;
    return (pal_word >> shift) & 0xFFFFu;
}

// ─── Vertex helpers ─────────────────────────────────────────────────────

// Pack normal + material into u32. Normal is axis-aligned (snorm8).
fn pack_normal_material(face: u32, mat_id: u32) -> u32 {
    // snorm8: +1.0 = 0x7F (127), -1.0 = 0x81 (-127 as i8, 129 as u8)
    switch face {
        case 0u: { return 0x00007F00u | (mat_id << 24u); } // +Y: ny=+1
        case 1u: { return 0x00008100u | (mat_id << 24u); } // -Y: ny=-1
        case 2u: { return 0x0000007Fu | (mat_id << 24u); } // +X: nx=+1
        case 3u: { return 0x00000081u | (mat_id << 24u); } // -X: nx=-1
        case 4u: { return 0x007F0000u | (mat_id << 24u); } // +Z: nz=+1
        default: { return 0x00810000u | (mat_id << 24u); } // -Z: nz=-1
    }
}

// Write a vertex (4 u32: 3 f32 position + 1 u32 normal_material) to vertex_pool.
fn write_vertex(base: u32, px: f32, py: f32, pz: f32, nm: u32) {
    vertex_pool[base]      = bitcast<u32>(px);
    vertex_pool[base + 1u] = bitcast<u32>(py);
    vertex_pool[base + 2u] = bitcast<u32>(pz);
    vertex_pool[base + 3u] = nm;
}

// ─── Private bitmaps (62×62 bits each in private memory) ────────────────
//
// PERFORMANCE NOTE (F6): Two 121-u32 private bitmaps (visible + processed)
// = 968 bytes per thread, exceeding GPU register file capacity (~1 KB).
// Both spill to VRAM via scratch memory. Each bitmap access is a cached
// global memory load (~100 cycles). Phase 5 optimization: tiled 8×8
// sub-processing with register-resident bitmaps (8 bytes per tile).

const BITMAP_WORDS: u32 = 121u; // ceil(62*62 / 32)

// Processed bitmap — tracks which (primary, secondary) cells have been merged.
var<private> processed: array<u32, 121>;

// Visibility bitmap (F2) — pre-computed face visibility for this slice.
// Set once before the merge loop. Eliminates redundant cull_column calls
// during width/height extension. See: reference/gpu-greedy-mesher-review.md F2.
var<private> visible: array<u32, 121>;

fn bitmap_get(bmp: ptr<private, array<u32, 121>>, a: u32, b: u32) -> bool {
    let idx = a * CS + b;
    let word = idx >> 5u;
    let bit = idx & 31u;
    return ((*bmp)[word] >> bit & 1u) != 0u;
}

fn bitmap_set(bmp: ptr<private, array<u32, 121>>, a: u32, b: u32) {
    let idx = a * CS + b;
    let word = idx >> 5u;
    let bit = idx & 31u;
    (*bmp)[word] |= 1u << bit;
}

fn bitmap_clear(bmp: ptr<private, array<u32, 121>>) {
    for (var i = 0u; i < BITMAP_WORDS; i++) {
        (*bmp)[i] = 0u;
    }
}

// ─── Entry point ────────────────────────────────────────────────────────

@compute @workgroup_size(64, 1, 1)
fn main(
    @builtin(workgroup_id) wg_id: vec3u,
    @builtin(local_invocation_id) local_id: vec3u,
) {
    let slot = wg_id.x;
    let face = wg_id.y;
    let slice = local_id.x;

    if slice >= CS {
        return;
    }

    let slot_offset = slot * WORDS_PER_SLOT;
    let chunk_coord = coord[slot];
    let vs = scene_params.w;   // voxel_size
    let go = scene_params.xyz; // grid_origin
    // Global voxel base: chunk_coord * CS - 1.
    // Vertex positions are computed as f32(global_int) * vs + go to ensure
    // identical floating-point results at chunk boundaries (no seams).
    // See: docs/Resident Representation/chunk-world-offset-analysis.md
    let chunk_base = vec3i(
        chunk_coord.x * i32(CS) - 1,
        chunk_coord.y * i32(CS) - 1,
        chunk_coord.z * i32(CS) - 1,
    );

    // Per-slot vertex/index pool offsets (variable allocation from prefix sum)
    // mesh_offset_table layout: 5 u32 per slot [vert_offset, vert_count, idx_offset, idx_count, write_counter]
    let ot_base = slot * 5u;
    let alloc_vert_offset = atomicLoad(&mesh_offset_table[ot_base]);
    let alloc_vert_count  = atomicLoad(&mesh_offset_table[ot_base + 1u]);
    let alloc_idx_offset  = atomicLoad(&mesh_offset_table[ot_base + 2u]);
    let alloc_idx_count   = atomicLoad(&mesh_offset_table[ot_base + 3u]);
    let slot_vert_base = alloc_vert_offset * VERTEX_STRIDE;
    let slot_idx_base = alloc_idx_offset;

    // Read bits_per_entry once per thread (uniform across all voxels in the slot).
    // palette_meta is 2 × u32 per slot: [0]=palette_size|bpe|reserved, [1]=index_buf_offset
    let bpe = (palette_meta[slot * 2u] >> 16u) & 0xFFu;

    bitmap_clear(&processed);
    bitmap_clear(&visible);

    // ── F2: Pre-compute visibility bitmap ──────────────────────────────
    // One pass over all 62×62 positions in this slice. Each position is culled
    // once (2 global reads for column + 2 for neighbor + bit math). The merge
    // loop then reads from the bitmap (private memory) instead of re-culling.
    // Eliminates ~6,000-10,000 redundant global memory reads per thread.

    for (var p = 0u; p < CS; p++) {
        for (var s = 0u; s < CS; s++) {
            var px: u32; var pz: u32; var y_bit: u32;
            switch face {
                case 0u, 1u: { px = p + 1u; pz = s + 1u; y_bit = slice; }
                case 2u, 3u: { px = slice + 1u; pz = s + 1u; y_bit = p; }
                default:     { px = p + 1u; pz = slice + 1u; y_bit = s; }
            }
            let col = read_col(slot_offset, px, pz);
            if col.x == 0u && col.y == 0u { continue; }
            let nbr = get_neighbor(slot_offset, px, pz, face);
            let fm = cull_column(col, nbr, face);
            if bit_set(fm, y_bit) {
                bitmap_set(&visible, p, s);
            }
        }
    }

    // ── Greedy merge using pre-computed visibility ──────────────────────
    // The sweep axis and scan plane depend on face direction.
    // For Y faces (0,1): sweep Y (slice = usable y), scan X×Z
    // For X faces (2,3): sweep X (slice = usable x), scan Y×Z
    // For Z faces (4,5): sweep Z (slice = usable z), scan X×Y

    for (var primary = 0u; primary < CS; primary++) {
        for (var secondary = 0u; secondary < CS; secondary++) {
            if bitmap_get(&processed, primary, secondary) {
                continue;
            }
            if !bitmap_get(&visible, primary, secondary) {
                continue;
            }

            // ── Resolve seed material ──
            // Map (face, slice, primary, secondary) → padded (px, py, pz)
            var seed_px: u32; var seed_py: u32; var seed_pz: u32;
            switch face {
                case 0u, 1u: { seed_px = primary + 1u; seed_py = slice + 1u; seed_pz = secondary + 1u; }
                case 2u, 3u: { seed_px = slice + 1u; seed_py = primary + 1u; seed_pz = secondary + 1u; }
                default:     { seed_px = primary + 1u; seed_py = secondary + 1u; seed_pz = slice + 1u; }
            }
            let seed_mat = read_material_id(slot, seed_px, seed_py, seed_pz, bpe);

            // ── Extend width (primary direction) ──
            var width = 1u;
            loop {
                if primary + width >= CS { break; }
                if bitmap_get(&processed, primary + width, secondary) { break; }
                if !bitmap_get(&visible, primary + width, secondary) { break; }
                // Material boundary check
                var cand_px: u32; var cand_py: u32; var cand_pz: u32;
                switch face {
                    case 0u, 1u: { cand_px = primary + width + 1u; cand_py = slice + 1u; cand_pz = secondary + 1u; }
                    case 2u, 3u: { cand_px = slice + 1u; cand_py = primary + width + 1u; cand_pz = secondary + 1u; }
                    default:     { cand_px = primary + width + 1u; cand_py = secondary + 1u; cand_pz = slice + 1u; }
                }
                if read_material_id(slot, cand_px, cand_py, cand_pz, bpe) != seed_mat { break; }
                width++;
            }

            // ── Extend height (secondary direction) ──
            var height = 1u;
            var height_done = false;
            loop {
                if secondary + height >= CS || height_done { break; }
                let ns = secondary + height;
                for (var dw = 0u; dw < width; dw++) {
                    let cp = primary + dw;
                    if bitmap_get(&processed, cp, ns) { height_done = true; break; }
                    if !bitmap_get(&visible, cp, ns) { height_done = true; break; }
                    // Material boundary check
                    var h_px: u32; var h_py: u32; var h_pz: u32;
                    switch face {
                        case 0u, 1u: { h_px = cp + 1u; h_py = slice + 1u; h_pz = ns + 1u; }
                        case 2u, 3u: { h_px = slice + 1u; h_py = cp + 1u; h_pz = ns + 1u; }
                        default:     { h_px = cp + 1u; h_py = ns + 1u; h_pz = slice + 1u; }
                    }
                    if read_material_id(slot, h_px, h_py, h_pz, bpe) != seed_mat { height_done = true; break; }
                }
                if !height_done { height++; }
            }

            // ── Mark processed ──
            for (var dw = 0u; dw < width; dw++) {
                for (var dh = 0u; dh < height; dh++) {
                    bitmap_set(&processed, primary + dw, secondary + dh);
                }
            }

            // ── Emit quad: 4 vertices + 6 indices ──

            let nm = pack_normal_material(face, seed_mat);

            // Claim one quad's worth of space via atomic counter in offset table.
            // write_counter (index 4 in per-slot block) counts quads claimed so far.
            let quad_claim = atomicAdd(&mesh_offset_table[ot_base + 4u], 1u);
            let vert_claim = quad_claim * 4u;
            let idx_claim = quad_claim * 6u;

            // Safety check: should not overflow if count pass was correct.
            if vert_claim + 4u > alloc_vert_count || idx_claim + 6u > alloc_idx_count {
                continue;
            }

            // Compute world-space quad corners.
            // EVERY vertex is computed from its own integer global coordinate:
            //   world = f32(chunk_base + local_int) * vs + go
            // This ensures vertices at chunk boundaries are bit-identical (no seams).
            // We never add float width/height offsets — each corner is independent.

            let cb = chunk_base;
            let p1 = i32(primary + 1u);
            let s1 = i32(secondary + 1u);
            let sl1 = i32(slice + 1u);
            let wi = i32(width);
            let hi = i32(height);

            // Helper: convert integer global voxel coord to world float
            // Inlined per-axis to avoid struct overhead in the hot loop.

            switch face {
                case 0u: { // +Y: face at y=slice+2, sweep X(w)×Z(h)
                    let x0 = f32(cb.x + p1) * vs + go.x;
                    let x1 = f32(cb.x + p1 + wi) * vs + go.x;
                    let y0 = f32(cb.y + sl1 + 1) * vs + go.y;
                    let z0 = f32(cb.z + s1) * vs + go.z;
                    let z1 = f32(cb.z + s1 + hi) * vs + go.z;
                    let vb = slot_vert_base + vert_claim * VERTEX_STRIDE;
                    write_vertex(vb,       x0, y0, z0, nm);
                    write_vertex(vb + 4u,  x0, y0, z1, nm);
                    write_vertex(vb + 8u,  x1, y0, z1, nm);
                    write_vertex(vb + 12u, x1, y0, z0, nm);
                }
                case 1u: { // -Y: face at y=slice+1, sweep X(w)×Z(h)
                    let x0 = f32(cb.x + p1) * vs + go.x;
                    let x1 = f32(cb.x + p1 + wi) * vs + go.x;
                    let y0 = f32(cb.y + sl1) * vs + go.y;
                    let z0 = f32(cb.z + s1) * vs + go.z;
                    let z1 = f32(cb.z + s1 + hi) * vs + go.z;
                    let vb = slot_vert_base + vert_claim * VERTEX_STRIDE;
                    write_vertex(vb,       x0, y0, z0, nm);
                    write_vertex(vb + 4u,  x1, y0, z0, nm);
                    write_vertex(vb + 8u,  x1, y0, z1, nm);
                    write_vertex(vb + 12u, x0, y0, z1, nm);
                }
                case 2u: { // +X: face at x=slice+2, sweep Y(w)×Z(h)
                    let x0 = f32(cb.x + sl1 + 1) * vs + go.x;
                    let y0 = f32(cb.y + p1) * vs + go.y;
                    let y1 = f32(cb.y + p1 + wi) * vs + go.y;
                    let z0 = f32(cb.z + s1) * vs + go.z;
                    let z1 = f32(cb.z + s1 + hi) * vs + go.z;
                    let vb = slot_vert_base + vert_claim * VERTEX_STRIDE;
                    write_vertex(vb,       x0, y0, z0, nm);
                    write_vertex(vb + 4u,  x0, y1, z0, nm);
                    write_vertex(vb + 8u,  x0, y1, z1, nm);
                    write_vertex(vb + 12u, x0, y0, z1, nm);
                }
                case 3u: { // -X: face at x=slice+1, sweep Y(w)×Z(h)
                    let x0 = f32(cb.x + sl1) * vs + go.x;
                    let y0 = f32(cb.y + p1) * vs + go.y;
                    let y1 = f32(cb.y + p1 + wi) * vs + go.y;
                    let z0 = f32(cb.z + s1) * vs + go.z;
                    let z1 = f32(cb.z + s1 + hi) * vs + go.z;
                    let vb = slot_vert_base + vert_claim * VERTEX_STRIDE;
                    write_vertex(vb,       x0, y0, z0, nm);
                    write_vertex(vb + 4u,  x0, y0, z1, nm);
                    write_vertex(vb + 8u,  x0, y1, z1, nm);
                    write_vertex(vb + 12u, x0, y1, z0, nm);
                }
                case 4u: { // +Z: face at z=slice+2, sweep X(w)×Y(h)
                    let x0 = f32(cb.x + p1) * vs + go.x;
                    let x1 = f32(cb.x + p1 + wi) * vs + go.x;
                    let y0 = f32(cb.y + s1) * vs + go.y;
                    let y1 = f32(cb.y + s1 + hi) * vs + go.y;
                    let z0 = f32(cb.z + sl1 + 1) * vs + go.z;
                    let vb = slot_vert_base + vert_claim * VERTEX_STRIDE;
                    write_vertex(vb,       x0, y0, z0, nm);
                    write_vertex(vb + 4u,  x1, y0, z0, nm);
                    write_vertex(vb + 8u,  x1, y1, z0, nm);
                    write_vertex(vb + 12u, x0, y1, z0, nm);
                }
                default: { // -Z: face at z=slice+1, sweep X(w)×Y(h)
                    let x0 = f32(cb.x + p1) * vs + go.x;
                    let x1 = f32(cb.x + p1 + wi) * vs + go.x;
                    let y0 = f32(cb.y + s1) * vs + go.y;
                    let y1 = f32(cb.y + s1 + hi) * vs + go.y;
                    let z0 = f32(cb.z + sl1) * vs + go.z;
                    let vb = slot_vert_base + vert_claim * VERTEX_STRIDE;
                    write_vertex(vb,       x0, y0, z0, nm);
                    write_vertex(vb + 4u,  x0, y1, z0, nm);
                    write_vertex(vb + 8u,  x1, y1, z0, nm);
                    write_vertex(vb + 12u, x1, y0, z0, nm);
                }
            }

            // Write 6 indices: [0,1,2, 0,2,3] pattern (CCW winding, both triangles face outward)
            let ib = slot_idx_base + idx_claim;
            let vbase = vert_claim;
            index_pool[ib]      = vbase;
            index_pool[ib + 1u] = vbase + 1u;
            index_pool[ib + 2u] = vbase + 2u;
            index_pool[ib + 3u] = vbase;
            index_pool[ib + 4u] = vbase + 2u;
            index_pool[ib + 5u] = vbase + 3u;
        }
    }
}
