// v3 Phase A cascade build kernel.
//
// Writes outgoing radiance + opacity into `probe_payload` for every
// (probe, direction) pair across all currently-active v3 probe slots.
//
// Dispatch shape (computed in dispatch.rs):
//   workgroup_size = (8, 8, 1)
//   dispatch       = (16, 16, 16 * V3_MAX_PROBE_SLOTS)
//
// One *workgroup* per probe — 16³ = 4096 probes/chunk × 64 chunks = 262144
// workgroups. One *thread* per octahedral direction texel within that
// workgroup (8 × 8 = 64 dirs/probe). The probe slot index is encoded in the
// upper bits of `workgroup_id.z`:
//
//   probe_x       = workgroup_id.x          (0..15)
//   probe_y       = workgroup_id.y          (0..15)
//   probe_z       = workgroup_id.z % 16     (0..15)
//   v3_probe_slot = workgroup_id.z / 16     (0..V3_MAX_PROBE_SLOTS)
//   dir_x         = local_invocation_id.x   (0..7)
//   dir_y         = local_invocation_id.y   (0..7)
//
// The kernel uses `workgroup_id`, NOT `global_invocation_id`, because
// global_invocation_id mixes the workgroup and local IDs in a way that
// breaks the probe-vs-direction split — see the bug investigation notes
// in radiance-cascades-symptoms.md (Phase A "ghost probes" issue).
//
// Unallocated probe slots are detected via `active_probe_chunks[v3_probe_slot].w == -1`
// and early-out, so the dispatch shape is constant regardless of how many
// chunks are actually resident. Phase A correctness > Phase A performance;
// Phase D will batch dispatches to skip unallocated work entirely.

// Prepended at runtime by dispatch.rs:
//   1. dda_common.wgsl
//   2. cascade_common.wgsl

// ─── Bindings ───────────────────────────────────────────────────────────

// Group 0 — World data (mirrors v2's group 0 layout)
@group(0) @binding(0) var<storage, read> occupancy:         array<u32>;
@group(0) @binding(1) var<storage, read> flags:             array<u32>;
@group(0) @binding(2) var<storage, read> slot_table:        array<u32>;
@group(0) @binding(3) var<storage, read> material_table:    array<vec4u>;
@group(0) @binding(4) var<storage, read> palette:           array<u32>;
@group(0) @binding(5) var<storage, read> palette_meta:      array<u32>;
@group(0) @binding(6) var<storage, read> index_buf_pool:    array<u32>;
@group(0) @binding(7) var<uniform>       slot_table_params: vec4i;

// Group 1 — v3 cascade data (write target)
@group(1) @binding(0) var<storage, read_write> probe_payload:        array<vec4f>;
@group(1) @binding(1) var<storage, read>       probe_slot_table:     array<u32>;
@group(1) @binding(2) var<uniform>             cascade_params:       V3CascadeParams;
/// Reverse lookup: index `i` = i-th allocated v3 probe slot →
/// (chunk_x, chunk_y, chunk_z, chunk_slot). Written by CPU each time
/// the slot table is updated. Avoids the O(N²) scan that the kernel
/// would otherwise need to invert the slot table per thread.
@group(1) @binding(3) var<storage, read>       active_probe_chunks:  array<vec4i>;

// ─── Sun + sky constants (Phase A: hardcoded, matches v2) ───────────────

const V3_SUN_DIR: vec3f          = vec3f(0.242, 0.647, 0.404);
const V3_SUN_RADIANCE: vec3f     = vec3f(3.0, 2.85, 2.55);
const V3_SKY_COLOR: vec3f        = vec3f(0.0, 0.0, 0.0);
const V3_GROUND_COLOR: vec3f     = vec3f(0.0, 0.0, 0.0);
const V3_SHADOW_MAX_DIST: f32    = 128.0;
const V3_PI: f32                 = 3.14159265;

// Maximum DDA trace distance for primary cascade-0 rays (voxel units).
//
// Phase A has only one cascade, so this single value must cover the
// longest expected ray inside a typical chunk. For a 62-voxel chunk,
// the longest in-chunk ray is ~107 voxels (the diagonal). 128 gives
// headroom for cross-chunk continuation up to ~2 chunks of distance.
//
// Phase B will introduce the cascade hierarchy where cascade 0 reverts
// to a true near-field reach (~4 voxels) and higher cascades take over
// the longer ranges via the geometric series.
const V3_PRIMARY_MAX_DIST: f32   = 128.0;

// Tiny offset away from the probe origin so DDA doesn't immediately self-hit
// the voxel the probe sits inside (for probes that happen to land in solid).
const V3_RAY_EPSILON: f32        = 0.01;

// ─── Material lookups (ported verbatim from v2) ─────────────────────────

fn v3_lookup_emissive(slot: u32, local: vec3u) -> vec3f {
    let meta0 = palette_meta[slot * 2u];
    let bpe = (meta0 >> 16u) & 0xFFu;
    if (bpe == 0u) { return vec3f(0.0); }
    let flat_idx = local.x * DDA_CS_P * DDA_CS_P + local.y * DDA_CS_P + local.z;
    let ib_word_offset = palette_meta[slot * 2u + 1u];
    let bit_offset = flat_idx * bpe;
    let word_idx = ib_word_offset + bit_offset / 32u;
    let bit_pos = bit_offset & 31u;
    let mask = (1u << bpe) - 1u;
    var palette_idx = (index_buf_pool[word_idx] >> bit_pos) & mask;
    if (bit_pos + bpe > 32u) {
        let w1 = index_buf_pool[word_idx + 1u];
        palette_idx = palette_idx | ((w1 & ((1u << (bpe - (32u - bit_pos))) - 1u)) << (32u - bit_pos));
    }
    let pal_word = palette[slot * 128u + palette_idx / 2u];
    let mat_id = select(pal_word & 0xFFFFu, (pal_word >> 16u) & 0xFFFFu, (palette_idx & 1u) != 0u);
    let entry = material_table[mat_id];
    let emissive_rg = unpack2x16float(entry.z);
    let emissive_b_op = unpack2x16float(entry.w);
    return vec3f(emissive_rg.x, emissive_rg.y, emissive_b_op.x);
}

fn v3_lookup_albedo(slot: u32, local: vec3u) -> vec3f {
    let meta0 = palette_meta[slot * 2u];
    let bpe = (meta0 >> 16u) & 0xFFu;
    if (bpe == 0u) { return vec3f(0.5); }
    let flat_idx = local.x * DDA_CS_P * DDA_CS_P + local.y * DDA_CS_P + local.z;
    let ib_word_offset = palette_meta[slot * 2u + 1u];
    let bit_offset = flat_idx * bpe;
    let word_idx = ib_word_offset + bit_offset / 32u;
    let bit_pos = bit_offset & 31u;
    let mask = (1u << bpe) - 1u;
    var palette_idx = (index_buf_pool[word_idx] >> bit_pos) & mask;
    if (bit_pos + bpe > 32u) {
        let w1 = index_buf_pool[word_idx + 1u];
        palette_idx = palette_idx | ((w1 & ((1u << (bpe - (32u - bit_pos))) - 1u)) << (32u - bit_pos));
    }
    let pal_word = palette[slot * 128u + palette_idx / 2u];
    let mat_id = select(pal_word & 0xFFFFu, (pal_word >> 16u) & 0xFFFFu, (palette_idx & 1u) != 0u);
    let entry = material_table[mat_id];
    let albedo_rg = unpack2x16float(entry.x);
    let albedo_b_rough = unpack2x16float(entry.y);
    return vec3f(albedo_rg.x, albedo_rg.y, albedo_b_rough.x);
}

// ─── Hit radiance (Phase A: emissive + sun + hemisphere ambient) ────────
//
// Computes outgoing radiance from a hit surface. Multi-bounce feedback is
// deferred to Phase C; the sun and sky constants match v2's so the
// intensity scale is comparable during dual-mode validation.

fn v3_compute_hit_radiance(
    hit_pos: vec3f,
    hit_normal: vec3f,
    albedo: vec3f,
    emissive: vec3f,
) -> vec3f {
    var radiance = emissive;

    // Direct sun with shadow trace.
    let ndotl = max(dot(hit_normal, V3_SUN_DIR), 0.0);
    if (ndotl > 0.001) {
        let shadow_origin = hit_pos + hit_normal * 0.5;
        let shadow_hit = dda_trace_first_hit(shadow_origin, V3_SUN_DIR, V3_SHADOW_MAX_DIST);
        if (!shadow_hit.hit) {
            radiance += albedo * V3_SUN_RADIANCE * ndotl / V3_PI;
        }
    }

    // Hemisphere ambient (sky above, ground below).
    let hemisphere = mix(V3_GROUND_COLOR, V3_SKY_COLOR, hit_normal.y * 0.5 + 0.5);
    radiance += hemisphere * albedo * 0.5;

    return radiance;
}

// ─── Main ───────────────────────────────────────────────────────────────

@compute @workgroup_size(8, 8, 1)
fn main(
    @builtin(workgroup_id) wg_id: vec3u,
    @builtin(local_invocation_id) lid: vec3u,
) {
    // Decode work item — ONE WORKGROUP per probe, ONE THREAD per direction:
    //   wg_id.x, wg_id.y     = probe local x, y within chunk (0..15)
    //   wg_id.z              = probe_z + 16 * v3_probe_slot
    //   lid.x, lid.y         = direction texel x, y (0..7)
    //
    // Dispatch shape from dispatch.rs:
    //   workgroup_size = (8, 8, 1)
    //   dispatch       = (V3_PROBES_PER_CHUNK_AXIS,                  // 16
    //                     V3_PROBES_PER_CHUNK_AXIS,                  // 16
    //                     V3_PROBES_PER_CHUNK_AXIS * V3_MAX_PROBE_SLOTS) // 16 * 64 = 1024
    //
    // So one workgroup runs 64 threads (one per direction texel) and writes
    // a single probe's full 64-direction payload. Total workgroups =
    // 16 * 16 * 1024 = 262144 = 4096 probes/chunk * 64 chunks. ✓
    let probes_per_axis = V3_PROBES_PER_CHUNK_AXIS;
    let dirs_per_axis = V3_CASCADE_0_DIRS_PER_AXIS;

    let probe_local = vec3u(wg_id.x, wg_id.y, wg_id.z % probes_per_axis);
    let v3_probe_slot = wg_id.z / probes_per_axis;

    if (v3_probe_slot >= cascade_params.max_probe_slots) { return; }

    // Look up the chunk this probe slot belongs to. CPU writes
    // active_probe_chunks[v3_probe_slot] = (chunk_x, chunk_y, chunk_z, chunk_slot)
    // every time the chunk pool's residency changes. Unallocated probe
    // slots have w = -1.
    let entry = active_probe_chunks[v3_probe_slot];
    if (entry.w < 0) { return; }
    let chunk = entry.xyz;

    // World voxel position of this probe.
    let probe_world_voxel = v3_probe_world_voxel(chunk, probe_local);

    // Direction for this thread.
    let dir_idx = lid.x + lid.y * dirs_per_axis;
    let dir = v3_oct_decode_texel(lid.x, lid.y, dirs_per_axis);

    // Cast a ray from the probe along this direction.
    let ray_origin = probe_world_voxel + dir * V3_RAY_EPSILON;
    let hit = dda_trace_first_hit(ray_origin, dir, V3_PRIMARY_MAX_DIST);

    var radiance = vec3f(0.0);
    var opacity = 0.0;

    if (hit.hit) {
        opacity = 1.0;
        let hit_slot = dda_chunk_to_slot(hit.chunk);
        if (hit_slot != DDA_SENTINEL) {
            let local = dda_world_to_local(hit.voxel, hit.chunk);
            let emissive = v3_lookup_emissive(hit_slot, local);
            let albedo = v3_lookup_albedo(hit_slot, local);
            let hit_normal = vec3f(hit.face);
            let hit_pos = vec3f(hit.voxel) + 0.5;
            radiance = v3_compute_hit_radiance(hit_pos, hit_normal, albedo, emissive);
        }
    }

    // Write to the payload SSBO.
    let probe_idx = v3_probe_index_in_chunk(probe_local);
    let texel_idx = v3_payload_texel_index(v3_probe_slot, probe_idx, dir_idx);
    probe_payload[texel_idx] = vec4f(radiance, opacity);
}
