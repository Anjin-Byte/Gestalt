// I-3: Summary Rebuild Compute Shader
//
// One workgroup per chunk slot. 256 threads, each handles 16 columns.
// Reads occupancy atlas → writes occupancy summary, chunk flags, chunk AABB.
//
// See: docs/Resident Representation/stages/I-3-summary-rebuild.md

// ─── Constants ──────────────────────────────────────────────────────────

const CS_P: u32 = 64u;
const CS: u32 = 62u;
const COLUMNS_PER_CHUNK: u32 = 4096u;   // CS_P × CS_P
const WORDS_PER_COLUMN: u32 = 2u;       // u64 stored as 2 × u32
const WORDS_PER_SLOT: u32 = 8192u;      // COLUMNS_PER_CHUNK × 2
const BRICKLET_DIM: u32 = 8u;
const BRICKLETS_PER_AXIS: u32 = 8u;     // CS_P / BRICKLET_DIM
const SUMMARY_WORDS: u32 = 16u;         // 512 bits / 32

// Flag bit positions
const FLAG_IS_EMPTY: u32 = 1u;          // bit 0
const FLAG_IS_FULLY_OPAQUE: u32 = 2u;   // bit 1
const FLAG_HAS_EMISSIVE: u32 = 4u;      // bit 2
const FLAG_IS_RESIDENT: u32 = 8u;       // bit 3

const THREADS: u32 = 256u;
const COLS_PER_THREAD: u32 = 16u;       // 4096 / 256

// ─── Bindings ───────────────────────────────────────────────────────────

@group(0) @binding(0) var<storage, read>       occupancy:      array<u32>;
@group(0) @binding(1) var<storage, read>       palette:        array<u32>;
@group(0) @binding(2) var<storage, read>       coord:          array<vec4i>;
@group(0) @binding(3) var<storage, read>       material_table: array<vec4u>;
@group(0) @binding(4) var<storage, read_write> summary_out:    array<u32>;
@group(0) @binding(5) var<storage, read_write> flags_out:      array<u32>;
@group(0) @binding(6) var<storage, read_write> aabb_out:       array<vec4f>;
@group(0) @binding(7) var<uniform>             scene_params:   vec4f; // xyz=grid_origin, w=voxel_size

// ─── Shared memory ──────────────────────────────────────────────────────

var<workgroup> s_bricklets: array<atomic<u32>, 16>;
var<workgroup> s_min_x: atomic<i32>;
var<workgroup> s_min_y: atomic<i32>;
var<workgroup> s_min_z: atomic<i32>;
var<workgroup> s_max_x: atomic<i32>;
var<workgroup> s_max_y: atomic<i32>;
var<workgroup> s_max_z: atomic<i32>;
var<workgroup> s_popcount: atomic<u32>;
var<workgroup> s_has_emissive: atomic<u32>;

// ─── Entry point ────────────────────────────────────────────────────────

@compute @workgroup_size(256, 1, 1)
fn main(
    @builtin(workgroup_id) wg_id: vec3u,
    @builtin(local_invocation_id) local_id: vec3u,
) {
    let slot = wg_id.x;
    let tid = local_id.x;
    let slot_offset = slot * WORDS_PER_SLOT;

    // ── Initialize shared memory (first 16 threads clear bricklets) ──

    if tid < SUMMARY_WORDS {
        atomicStore(&s_bricklets[tid], 0u);
    }
    if tid == 0u {
        atomicStore(&s_min_x, 64);
        atomicStore(&s_min_y, 64);
        atomicStore(&s_min_z, 64);
        atomicStore(&s_max_x, -1);
        atomicStore(&s_max_y, -1);
        atomicStore(&s_max_z, -1);
        atomicStore(&s_popcount, 0u);
        atomicStore(&s_has_emissive, 0u);
    }

    workgroupBarrier();

    // ── Each thread processes 16 columns ──

    let col_start = tid * COLS_PER_THREAD;
    var thread_popcount = 0u;

    for (var i = 0u; i < COLS_PER_THREAD; i++) {
        let col = col_start + i;
        // Decompose column index into (x, z)
        let x = col / CS_P;
        let z = col % CS_P;

        let word_base = slot_offset + col * WORDS_PER_COLUMN;
        let lo = occupancy[word_base];
        let hi = occupancy[word_base + 1u];

        // Skip empty columns
        if lo == 0u && hi == 0u {
            continue;
        }

        thread_popcount += countOneBits(lo) + countOneBits(hi);

        // AABB: X and Z bounds
        atomicMin(&s_min_x, i32(x));
        atomicMax(&s_max_x, i32(x));
        atomicMin(&s_min_z, i32(z));
        atomicMax(&s_max_z, i32(z));

        // AABB: Y bounds via leading/trailing zeros on the u64
        // low_y = ctz(column), high_y = 63 - clz(column)
        var low_y: i32;
        var high_y: i32;
        if lo != 0u {
            low_y = i32(countTrailingZeros(lo));
        } else {
            low_y = i32(32u + countTrailingZeros(hi));
        }
        if hi != 0u {
            high_y = i32(63u - countLeadingZeros(hi));
        } else {
            high_y = i32(31u - countLeadingZeros(lo));
        }
        atomicMin(&s_min_y, low_y);
        atomicMax(&s_max_y, high_y);

        // Bricklet summary
        let bx = x / BRICKLET_DIM;
        let bz = z / BRICKLET_DIM;
        for (var by = 0u; by < BRICKLETS_PER_AXIS; by++) {
            let y_start = by * BRICKLET_DIM;
            // Extract 8 bits from the column at y_start
            var occupied: bool;
            if y_start < 32u {
                let end = y_start + 8u;
                if end <= 32u {
                    // Entirely in lo word
                    occupied = ((lo >> y_start) & 0xFFu) != 0u;
                } else {
                    // Spans lo/hi boundary
                    let lo_bits = lo >> y_start;
                    let hi_bits = hi << (32u - y_start);
                    occupied = ((lo_bits | hi_bits) & 0xFFu) != 0u;
                }
            } else {
                // Entirely in hi word
                let shift = y_start - 32u;
                occupied = ((hi >> shift) & 0xFFu) != 0u;
            }

            if occupied {
                let bit_index = bx * 64u + by * 8u + bz;
                let word_idx = bit_index >> 5u;
                let bit_within = bit_index & 31u;
                atomicOr(&s_bricklets[word_idx], 1u << bit_within);
            }
        }
    }

    // Accumulate popcount
    if thread_popcount > 0u {
        atomicAdd(&s_popcount, thread_popcount);
    }

    workgroupBarrier();

    // ── Thread 0: write outputs ──

    if tid == 0u {
        let total_pop = atomicLoad(&s_popcount);

        // Write summary
        let summary_offset = slot * SUMMARY_WORDS;
        for (var i = 0u; i < SUMMARY_WORDS; i++) {
            summary_out[summary_offset + i] = atomicLoad(&s_bricklets[i]);
        }

        // Build flags
        var f = FLAG_IS_RESIDENT;
        if total_pop == 0u {
            f |= FLAG_IS_EMPTY;
        }
        // is_fully_opaque: check if total popcount == 64^3 = 262144
        if total_pop == 262144u {
            f |= FLAG_IS_FULLY_OPAQUE;
        }
        if atomicLoad(&s_has_emissive) != 0u {
            f |= FLAG_HAS_EMISSIVE;
        }
        flags_out[slot] = f;

        // Write AABB in world space (scaled by voxel_size + grid_origin)
        let chunk_coord = coord[slot];
        let vs = scene_params.w;
        let go = scene_params.xyz;
        let world_offset = (vec3f(f32(chunk_coord.x), f32(chunk_coord.y), f32(chunk_coord.z)) * f32(CS) - vec3f(1.0)) * vs + go;

        if total_pop == 0u {
            aabb_out[slot * 2u] = vec4f(1e20, 1e20, 1e20, 0.0);
            aabb_out[slot * 2u + 1u] = vec4f(-1e20, -1e20, -1e20, 0.0);
        } else {
            let mn = vec3f(
                f32(atomicLoad(&s_min_x)),
                f32(atomicLoad(&s_min_y)),
                f32(atomicLoad(&s_min_z)),
            );
            let mx = vec3f(
                f32(atomicLoad(&s_max_x)),
                f32(atomicLoad(&s_max_y)),
                f32(atomicLoad(&s_max_z)),
            );
            aabb_out[slot * 2u] = vec4f(world_offset + mn * vs, 0.0);
            aabb_out[slot * 2u + 1u] = vec4f(world_offset + (mx + vec3f(1.0)) * vs, 0.0);
        }
    }

    // ── Emissive check (thread 0 only, after popcount is known) ──
    // Done separately since palette scan is sequential and only needed once.

    if tid == 0u {
        let total_pop = atomicLoad(&s_popcount);
        if total_pop > 0u {
            // Scan palette for emissive materials
            // Palette is packed u16 pairs. We check a reasonable max of 256 entries.
            for (var i = 0u; i < 256u; i++) {
                let word_idx = i / 2u;
                let shift = (i & 1u) * 16u;
                let mat_id = (palette[slot * 128u + word_idx] >> shift) & 0xFFFFu;
                if mat_id == 0u {
                    continue;
                }
                // material_table is array<vec4u>: [albedo_rg, albedo_b_roughness, emissive_rg, emissive_b_opacity]
                let entry = material_table[mat_id];
                if entry.z != 0u || (entry.w & 0xFFFFu) != 0u {
                    // Has emissive component
                    let old_flags = flags_out[slot];
                    flags_out[slot] = old_flags | FLAG_HAS_EMISSIVE;
                    break;
                }
            }
        }
    }
}
