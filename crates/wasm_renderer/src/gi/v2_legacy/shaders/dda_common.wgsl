// DDA Common — shared traversal functions for GI and debug passes.
//
// This file is prepended to consumer shaders via Rust-side string concat
// (WGSL has no #include). Consumers provide the bindings.
//
// Required bindings from consumer:
//   @group(?) @binding(?) var<storage, read> occupancy: array<u32>;
//   @group(?) @binding(?) var<storage, read> flags:     array<u32>;
//   @group(?) @binding(?) var<storage, read> slot_table: array<u32>;
//   @group(?) @binding(?) var<uniform>       slot_table_params: vec4i; // xyz=origin, w=dim
//
// See: docs/Resident Representation/traversal-acceleration.md

// ─── Constants ──────────────────────────────────────────────────────────

const DDA_CS_P: u32 = 64u;        // padded chunk dimension (storage)
const DDA_CS: u32 = 62u;          // usable chunk dimension (stride between chunks)
const DDA_WORDS_PER_SLOT: u32 = 8192u;
const DDA_SENTINEL: u32 = 0xFFFFFFFFu;
const DDA_MAX_STEPS: u32 = 512u;    // safety limit for chunk-level DDA
const DDA_MAX_VOXEL_STEPS: u32 = 192u; // safety limit per-chunk voxel DDA

const DDA_FLAG_IS_EMPTY: u32 = 1u;  // bit 0 of flags

// ─── Hit result ─────────────────────────────────────────────────────────

struct DdaHit {
    hit: bool,
    t: f32,
    voxel: vec3i,       // world voxel coordinate
    face: vec3i,        // face normal (axis-aligned, e.g. (0,1,0))
    chunk: vec3i,       // chunk that was being traversed when hit occurred
};

// ─── Slot table lookup ──────────────────────────────────────────────────

fn dda_chunk_to_slot(chunk: vec3i) -> u32 {
    let origin = slot_table_params.xyz;
    let dim = slot_table_params.w;
    let local = chunk - origin;
    if (any(local < vec3i(0)) || any(local >= vec3i(dim))) {
        return DDA_SENTINEL;
    }
    let idx = u32(local.x) + u32(local.y) * u32(dim) + u32(local.z) * u32(dim) * u32(dim);
    return slot_table[idx];
}

// ─── Occupancy bit test ─────────────────────────────────────────────────

fn dda_is_occupied(slot: u32, lx: u32, ly: u32, lz: u32) -> bool {
    let col_idx = lx * DDA_CS_P + lz;
    let base = slot * DDA_WORDS_PER_SLOT + col_idx * 2u;
    let word_idx = select(0u, 1u, ly >= 32u);
    let bit = ly & 31u;
    return ((occupancy[base + word_idx] >> bit) & 1u) != 0u;
}

// ─── Floor division (correct for negative coords) ──────────────────────

fn dda_floor_div(a: i32, b: i32) -> i32 {
    // Integer floor division: floor(a / b) for b > 0
    return select(a / b, (a - b + 1) / b, a < 0 && (a % b) != 0);
}

// ─── Chunk ↔ world voxel coordinate mapping ────────────────────────────
//
// Mesh shader computes: world_voxel = chunk_coord * CS - 1 + local
//   where local ∈ [0, 63] (the full padded 64³ grid)
//   and CS = 62 (usable dimension, stride between chunks)
//
// So chunk's world origin is at: chunk_coord * 62 - 1
// And the chunk contains world voxels: [chunk_coord*62 - 1, chunk_coord*62 + 62]
//
// To find which chunk a world voxel belongs to:
//   chunk_coord = floor((world_voxel + 1) / 62)

fn dda_world_to_chunk(world_voxel: vec3f) -> vec3i {
    let shifted = world_voxel + vec3f(1.0);
    let cs = f32(DDA_CS);
    return vec3i(
        i32(floor(shifted.x / cs)),
        i32(floor(shifted.y / cs)),
        i32(floor(shifted.z / cs)),
    );
}

fn dda_chunk_world_origin(chunk: vec3i) -> vec3f {
    // World-space voxel coordinate of local (0,0,0) in this chunk
    let cs = f32(DDA_CS);
    return vec3f(chunk) * cs - vec3f(1.0);
}

fn dda_world_to_local(world_voxel: vec3i, chunk: vec3i) -> vec3u {
    let origin = chunk * i32(DDA_CS) - vec3i(1);
    return vec3u(world_voxel - origin);
}

// ─── Voxel DDA inside a single chunk ────────────────────────────────────
//
// Steps through voxels within a 64³ chunk. Returns hit info or miss.
// entry_t is the ray t at the chunk entry point.

fn dda_voxel_trace(
    slot: u32,
    chunk_origin_world: vec3f,  // world-space origin of this chunk in voxels
    ray_origin: vec3f,          // world-space ray origin (in voxel units)
    ray_dir: vec3f,             // normalized direction
    t_enter: f32,
    t_max: f32,
) -> DdaHit {
    var result: DdaHit;
    result.hit = false;
    result.t = t_max;
    result.voxel = vec3i(0);
    result.face = vec3i(0);
    result.chunk = vec3i(0);

    // Compute entry point in local chunk space [0, 64)
    let entry_world = ray_origin + ray_dir * t_enter;
    let local_f = entry_world - chunk_origin_world;

    // Clamp to valid range [0, 63]
    var voxel = vec3i(
        clamp(i32(floor(local_f.x)), 0, 63),
        clamp(i32(floor(local_f.y)), 0, 63),
        clamp(i32(floor(local_f.z)), 0, 63),
    );

    // A&W step setup
    let step = vec3i(
        select(-1, 1, ray_dir.x >= 0.0),
        select(-1, 1, ray_dir.y >= 0.0),
        select(-1, 1, ray_dir.z >= 0.0),
    );

    let inv_dir = vec3f(
        select(1.0 / ray_dir.x, 1e30, abs(ray_dir.x) < 1e-10),
        select(1.0 / ray_dir.y, 1e30, abs(ray_dir.y) < 1e-10),
        select(1.0 / ray_dir.z, 1e30, abs(ray_dir.z) < 1e-10),
    );

    let t_delta = abs(inv_dir); // t to cross one voxel in each axis

    // t to next voxel boundary from the entry point
    let next_boundary = vec3f(voxel) + vec3f(select(0.0, 1.0, step.x > 0), select(0.0, 1.0, step.y > 0), select(0.0, 1.0, step.z > 0));
    var t_next = (next_boundary + chunk_origin_world - ray_origin) * inv_dir;

    // Check initial voxel
    if dda_is_occupied(slot, u32(voxel.x), u32(voxel.y), u32(voxel.z)) {
        result.hit = true;
        result.t = t_enter;
        result.voxel = voxel + vec3i(chunk_origin_world);
        // face = direction we entered from (approximation for initial voxel)
        result.face = -step;
        return result;
    }

    // Step through voxels
    for (var i = 0u; i < DDA_MAX_VOXEL_STEPS; i++) {
        // Find axis with smallest t_next (branchless)
        var axis = 0;
        if (t_next.y < t_next.x && t_next.y < t_next.z) { axis = 1; }
        else if (t_next.z < t_next.x) { axis = 2; }

        let t_crossing = select(select(t_next.z, t_next.y, axis == 1), t_next.x, axis == 0);

        // Advance past t_max → no hit in this chunk
        if (t_crossing > t_max) { return result; }

        // Step along chosen axis
        if (axis == 0) {
            voxel.x += step.x;
            t_next.x += t_delta.x;
        } else if (axis == 1) {
            voxel.y += step.y;
            t_next.y += t_delta.y;
        } else {
            voxel.z += step.z;
            t_next.z += t_delta.z;
        }

        // Exited chunk bounds → done with this chunk
        if (any(voxel < vec3i(0)) || any(voxel >= vec3i(64))) {
            return result;
        }

        // Test occupancy
        if dda_is_occupied(slot, u32(voxel.x), u32(voxel.y), u32(voxel.z)) {
            result.hit = true;
            result.t = t_crossing;
            result.voxel = voxel + vec3i(chunk_origin_world);
            // Face is the axis we stepped on, inverted (the side we entered from)
            result.face = vec3i(0);
            if (axis == 0) { result.face.x = -step.x; }
            else if (axis == 1) { result.face.y = -step.y; }
            else { result.face.z = -step.z; }
            return result;
        }
    }

    return result;
}

// ─── Two-level DDA: chunk → voxel ───────────────────────────────────────
//
// Chunk stride is CS=62 voxels (not CS_P=64). Chunk world origin = chunk*62-1.
// The chunk grid is on a 62-voxel stride, so chunk boundaries are at:
//   chunk*62 - 1           (local voxel 0, padded edge)
//   chunk*62 - 1 + 63      (local voxel 63, padded edge)
// Usable voxels: local [1, 62] → world [chunk*62, chunk*62 + 61]

fn dda_trace_first_hit(
    ray_origin: vec3f,   // in voxel-space
    ray_dir: vec3f,      // normalized direction
    t_max: f32,          // max distance in voxel units
) -> DdaHit {
    var result: DdaHit;
    result.hit = false;
    result.t = t_max;
    result.voxel = vec3i(0);
    result.face = vec3i(0);
    result.chunk = vec3i(0);

    // Find starting chunk from ray origin
    var chunk = dda_world_to_chunk(ray_origin);

    // A&W setup at chunk granularity (stride = CS = 62)
    let step = vec3i(
        select(-1, 1, ray_dir.x >= 0.0),
        select(-1, 1, ray_dir.y >= 0.0),
        select(-1, 1, ray_dir.z >= 0.0),
    );

    let inv_dir = vec3f(
        select(1.0 / ray_dir.x, 1e30, abs(ray_dir.x) < 1e-10),
        select(1.0 / ray_dir.y, 1e30, abs(ray_dir.y) < 1e-10),
        select(1.0 / ray_dir.z, 1e30, abs(ray_dir.z) < 1e-10),
    );

    let cs_f = f32(DDA_CS); // 62.0
    let t_delta_chunk = abs(inv_dir) * cs_f;

    // Chunk boundaries in world voxel space:
    //   chunk_start = chunk * 62 - 1  (padded origin)
    //   chunk_end   = chunk * 62 - 1 + 64 = chunk * 62 + 63
    // But for the DDA grid, the "cell" for chunk C covers [C*62-1, C*62+63).
    // Next boundary in step direction:
    let chunk_origin = dda_chunk_world_origin(chunk); // chunk*62 - 1
    let next_boundary = chunk_origin + vec3f(
        select(0.0, f32(DDA_CS_P), step.x > 0),
        select(0.0, f32(DDA_CS_P), step.y > 0),
        select(0.0, f32(DDA_CS_P), step.z > 0),
    );
    var t_next_chunk = (next_boundary - ray_origin) * inv_dir;
    var t_current = 0.0;

    for (var i = 0u; i < DDA_MAX_STEPS; i++) {
        if (t_current >= t_max) { break; }

        // Look up slot for this chunk
        let slot = dda_chunk_to_slot(chunk);

        if (slot != DDA_SENTINEL) {
            // Check is_empty flag
            let chunk_flags = flags[slot];
            if ((chunk_flags & DDA_FLAG_IS_EMPTY) == 0u) {
                // Descend to voxel-level DDA
                let c_origin = dda_chunk_world_origin(chunk);
                let t_enter = max(t_current, 0.0);
                let t_exit_chunk = min(min(t_next_chunk.x, min(t_next_chunk.y, t_next_chunk.z)), t_max);

                let voxel_hit = dda_voxel_trace(
                    slot,
                    c_origin,
                    ray_origin,
                    ray_dir,
                    t_enter,
                    t_exit_chunk,
                );

                if (voxel_hit.hit) {
                    var h = voxel_hit;
                    h.chunk = chunk;  // carry the traversal chunk for material lookup
                    return h;
                }
            }
        }

        // Advance to next chunk (branchless axis selection)
        var axis = 0;
        if (t_next_chunk.y < t_next_chunk.x && t_next_chunk.y < t_next_chunk.z) { axis = 1; }
        else if (t_next_chunk.z < t_next_chunk.x) { axis = 2; }

        t_current = select(select(t_next_chunk.z, t_next_chunk.y, axis == 1), t_next_chunk.x, axis == 0);

        if (axis == 0) {
            chunk.x += step.x;
            t_next_chunk.x += t_delta_chunk.x;
        } else if (axis == 1) {
            chunk.y += step.y;
            t_next_chunk.y += t_delta_chunk.y;
        } else {
            chunk.z += step.z;
            t_next_chunk.z += t_delta_chunk.z;
        }
    }

    return result;
}
