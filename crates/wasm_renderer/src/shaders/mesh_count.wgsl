// R-1 Pass 1: Mesh Count Compute Shader
//
// Counts quads per slot without emitting vertices/indices.
// Same face-cull + greedy-merge + material-aware algorithm as mesh_rebuild.wgsl.
// Output: mesh_counts[slot] = total quad count.
//
// Dispatch: (slot_count, 6, 1), @workgroup_size(64, 1, 1) — same as write pass.
//
// See: docs/Resident Representation/variable-mesh-pool.md

// ─── Constants ──────────────────────────────────────────────────────────

const CS_P: u32 = 64u;
const CS: u32 = 62u;
const COLUMNS_PER_CHUNK: u32 = 4096u;
const WORDS_PER_SLOT: u32 = 8192u;

const FACE_POS_Y: u32 = 0u;
const FACE_NEG_Y: u32 = 1u;
const FACE_POS_X: u32 = 2u;
const FACE_NEG_X: u32 = 3u;
const FACE_POS_Z: u32 = 4u;
const FACE_NEG_Z: u32 = 5u;

const USABLE_MASK_LO: u32 = 0xFFFFFFFFu;
const USABLE_MASK_HI: u32 = 0x3FFFFFFFu;

const PALETTE_WORDS_PER_SLOT: u32 = 128u;

// ─── Bindings ───────────────────────────────────────────────────────────

@group(0) @binding(0) var<storage, read>       occupancy:      array<u32>;
@group(0) @binding(1) var<storage, read>       palette:        array<u32>;
@group(0) @binding(2) var<storage, read>       coord:          array<vec4i>;
@group(0) @binding(3) var<storage, read_write> mesh_counts:    array<atomic<u32>>;
@group(0) @binding(4) var<storage, read>       index_buf_pool: array<u32>;
@group(0) @binding(5) var<storage, read>       palette_meta:   array<u32>;

// ─── Helpers (identical to mesh_rebuild.wgsl) ───────────────────────────

fn read_col(slot_offset: u32, x: u32, z: u32) -> vec2u {
    let col_idx = x * CS_P + z;
    let base = slot_offset + col_idx * 2u;
    return vec2u(occupancy[base], occupancy[base + 1u]);
}

fn shr1(v: vec2u) -> vec2u {
    let lo = (v.x >> 1u) | (v.y << 31u);
    let hi = v.y >> 1u;
    return vec2u(lo, hi);
}

fn shl1(v: vec2u) -> vec2u {
    let lo = v.x << 1u;
    let hi = (v.y << 1u) | (v.x >> 31u);
    return vec2u(lo, hi);
}

fn not2(v: vec2u) -> vec2u { return vec2u(~v.x, ~v.y); }
fn and2(a: vec2u, b: vec2u) -> vec2u { return vec2u(a.x & b.x, a.y & b.y); }

fn to_usable(v: vec2u) -> vec2u {
    let shifted = shr1(v);
    return vec2u(shifted.x & USABLE_MASK_LO, shifted.y & USABLE_MASK_HI);
}

fn bit_set(v: vec2u, y: u32) -> bool {
    if y < 32u { return (v.x >> y & 1u) != 0u; }
    else { return (v.y >> (y - 32u) & 1u) != 0u; }
}

fn cull_column(col: vec2u, neighbor: vec2u, face: u32) -> vec2u {
    switch face {
        case 0u: { return to_usable(and2(col, not2(shr1(col)))); }
        case 1u: { return to_usable(and2(col, not2(shl1(col)))); }
        default: { return to_usable(and2(col, not2(neighbor))); }
    }
}

fn get_neighbor(slot_offset: u32, x: u32, z: u32, face: u32) -> vec2u {
    switch face {
        case 2u: { return read_col(slot_offset, x + 1u, z); }
        case 3u: { return read_col(slot_offset, x - 1u, z); }
        case 4u: { return read_col(slot_offset, x, z + 1u); }
        case 5u: { return read_col(slot_offset, x, z - 1u); }
        default: { return vec2u(0u, 0u); }
    }
}

fn read_material_id(slot: u32, px: u32, py: u32, pz: u32, bpe: u32) -> u32 {
    let voxel_index = px * 4096u + py * 64u + pz;
    let bit_offset = voxel_index * bpe;
    let word_index = bit_offset >> 5u;
    let bit_within = bit_offset & 31u;
    let mask = (1u << bpe) - 1u;
    let slot_base = palette_meta[slot * 2u + 1u]; // index_buf_word_offset
    let palette_idx = (index_buf_pool[slot_base + word_index] >> bit_within) & mask;
    let pal_base = slot * PALETTE_WORDS_PER_SLOT;
    let pal_word = palette[pal_base + (palette_idx >> 1u)];
    let shift = (palette_idx & 1u) * 16u;
    return (pal_word >> shift) & 0xFFFFu;
}

// ─── Private bitmaps ────────────────────────────────────────────────────

const BITMAP_WORDS: u32 = 121u;
var<private> processed: array<u32, 121>;
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
    for (var i = 0u; i < BITMAP_WORDS; i++) { (*bmp)[i] = 0u; }
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

    if slice >= CS { return; }

    let slot_offset = slot * WORDS_PER_SLOT;
    let bpe = (palette_meta[slot * 2u] >> 16u) & 0xFFu;

    bitmap_clear(&processed);
    bitmap_clear(&visible);

    // ── Visibility bitmap precompute (identical to mesh_rebuild) ──

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

    // ── Greedy merge (identical logic, count only) ──────────────────────

    for (var primary = 0u; primary < CS; primary++) {
        for (var secondary = 0u; secondary < CS; secondary++) {
            if bitmap_get(&processed, primary, secondary) { continue; }
            if !bitmap_get(&visible, primary, secondary) { continue; }

            // Resolve seed material
            var seed_px: u32; var seed_py: u32; var seed_pz: u32;
            switch face {
                case 0u, 1u: { seed_px = primary + 1u; seed_py = slice + 1u; seed_pz = secondary + 1u; }
                case 2u, 3u: { seed_px = slice + 1u; seed_py = primary + 1u; seed_pz = secondary + 1u; }
                default:     { seed_px = primary + 1u; seed_py = secondary + 1u; seed_pz = slice + 1u; }
            }
            let seed_mat = read_material_id(slot, seed_px, seed_py, seed_pz, bpe);

            // Extend width
            var width = 1u;
            loop {
                if primary + width >= CS { break; }
                if bitmap_get(&processed, primary + width, secondary) { break; }
                if !bitmap_get(&visible, primary + width, secondary) { break; }
                var cand_px: u32; var cand_py: u32; var cand_pz: u32;
                switch face {
                    case 0u, 1u: { cand_px = primary + width + 1u; cand_py = slice + 1u; cand_pz = secondary + 1u; }
                    case 2u, 3u: { cand_px = slice + 1u; cand_py = primary + width + 1u; cand_pz = secondary + 1u; }
                    default:     { cand_px = primary + width + 1u; cand_py = secondary + 1u; cand_pz = slice + 1u; }
                }
                if read_material_id(slot, cand_px, cand_py, cand_pz, bpe) != seed_mat { break; }
                width++;
            }

            // Extend height
            var height = 1u;
            var height_done = false;
            loop {
                if secondary + height >= CS || height_done { break; }
                let ns = secondary + height;
                for (var dw = 0u; dw < width; dw++) {
                    let cp = primary + dw;
                    if bitmap_get(&processed, cp, ns) { height_done = true; break; }
                    if !bitmap_get(&visible, cp, ns) { height_done = true; break; }
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

            // Mark processed
            for (var dw = 0u; dw < width; dw++) {
                for (var dh = 0u; dh < height; dh++) {
                    bitmap_set(&processed, primary + dw, secondary + dh);
                }
            }

            // Count only — no vertex/index emission
            atomicAdd(&mesh_counts[slot], 1u);
        }
    }
}
