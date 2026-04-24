// v3 cascade common helpers — Phase A.
//
// Prepended via string concatenation to consumer shaders that need probe
// addressing, octahedral encode/decode, or chunk-slot lookups. Mirrors
// the CPU reference at `crates/wasm_renderer/src/gi/v3/reference.rs` —
// any change here must be applied to that file (and vice versa) and
// validated by the unit tests in `gi::v3::reference::tests`.
//
// Required bindings supplied by the consumer (varies by stage):
//   - probe_payload: array<vec4<f32>>     // Phase A storage layout
//   - probe_slot_table: array<u32>        // chunk_slot → probe_slot
//   - cascade_params: V3CascadeParams (uniform)
//
// Phase A uses `vec4<f32>` for the payload because WebGPU storage buffer
// access for f16 requires the `shader-f16` feature. Phase B may switch to
// packed f16 once the feature gate is enabled. The byte offset math
// in `flat_payload_texel_index` returns texel indices, not byte offsets,
// so the layout change is invisible to consumers as long as the texel
// stride matches the binding type.

// ─── Constants (must match gi::v3::constants) ───────────────────────────

const V3_CASCADE_0_SPACING: u32       = 4u;
const V3_CASCADE_0_DIRS_PER_AXIS: u32 = 8u;
const V3_CASCADE_0_DIRS: u32          = 64u;
const V3_PROBES_PER_CHUNK_AXIS: u32   = 16u;
const V3_PROBES_PER_CHUNK: u32        = 4096u;
const V3_MAX_PROBE_SLOTS: u32         = 64u;
const V3_PROBE_SENTINEL: u32          = 0xFFFFFFFFu;

// ─── V3CascadeParams (uniform layout) ────────────────────────────────────
// Mirrors `gi::v3::resources::V3CascadeParams`. 64 bytes, 16-byte aligned.

struct V3CascadeParams {
    grid_origin: vec3f,           // offset 0  (12 bytes)
    voxel_scale: f32,             // offset 12
    cascade_0_spacing: u32,       // offset 16
    dirs_per_axis: u32,           // offset 20
    probes_per_chunk_axis: u32,   // offset 24
    max_probe_slots: u32,         // offset 28
    frame_index: u32,             // offset 32
    active_probe_slots: u32,      // offset 36
    _pad0: u32,                   // offset 40
    _pad1: u32,                   // offset 44
    _pad2: u32,                   // offset 48
    _pad3: u32,                   // offset 52
    _pad4: u32,                   // offset 56
    _pad5: u32,                   // offset 60 — total 64 bytes
};

// ─── Octahedral encode / decode (full sphere) ────────────────────────────

fn v3_oct_decode(uv: vec2f) -> vec3f {
    let f = uv * 2.0 - vec2f(1.0, 1.0);
    var n = vec3f(f.x, 1.0 - abs(f.x) - abs(f.y), f.y);
    let t = max(-n.y, 0.0);
    n.x += select(t, -t, n.x >= 0.0);
    n.z += select(t, -t, n.z >= 0.0);
    return normalize(n);
}

fn v3_sign_not_zero(x: f32) -> f32 {
    return select(-1.0, 1.0, x >= 0.0);
}

fn v3_oct_encode(dir: vec3f) -> vec2f {
    let n = dir / (abs(dir.x) + abs(dir.y) + abs(dir.z));
    var octant: vec2f;
    if (n.y >= 0.0) {
        octant = vec2f(n.x, n.z);
    } else {
        octant = vec2f(
            (1.0 - abs(n.z)) * v3_sign_not_zero(n.x),
            (1.0 - abs(n.x)) * v3_sign_not_zero(n.z),
        );
    }
    return octant * 0.5 + vec2f(0.5, 0.5);
}

/// Decode the direction stored at integer octahedral texel `(dx, dy)` in
/// a `dirs_per_axis × dirs_per_axis` grid. Texel centers at (dx+0.5, dy+0.5).
fn v3_oct_decode_texel(dir_x: u32, dir_y: u32, dirs_per_axis: u32) -> vec3f {
    let uv = vec2f(
        (f32(dir_x) + 0.5) / f32(dirs_per_axis),
        (f32(dir_y) + 0.5) / f32(dirs_per_axis),
    );
    return v3_oct_decode(uv);
}

// ─── Probe addressing ────────────────────────────────────────────────────

/// Linearize a probe's local 3D coordinate within a chunk.
/// Order: x varies fastest, z slowest. Mirrors
/// `gi::v3::reference::probe_index_in_chunk`.
fn v3_probe_index_in_chunk(local_probe: vec3u) -> u32 {
    return local_probe.x
         + local_probe.y * V3_PROBES_PER_CHUNK_AXIS
         + local_probe.z * V3_PROBES_PER_CHUNK_AXIS * V3_PROBES_PER_CHUNK_AXIS;
}

/// Compute the flat texel index of a single direction texel within the
/// probe payload SSBO. Layout: slot-major, probe-major, dir-minor.
/// Mirrors `gi::v3::reference::flat_payload_texel_index`.
fn v3_payload_texel_index(probe_slot: u32, probe_idx: u32, dir_idx: u32) -> u32 {
    let slot_off = probe_slot * V3_PROBES_PER_CHUNK * V3_CASCADE_0_DIRS;
    let probe_off = probe_idx * V3_CASCADE_0_DIRS;
    return slot_off + probe_off + dir_idx;
}

/// Look up the v3 probe slot for a chunk slot index. Returns
/// `V3_PROBE_SENTINEL` if no probes are allocated for this chunk.
/// Caller must ensure `chunk_slot < pool::MAX_SLOTS`.
fn v3_chunk_slot_to_probe_slot(chunk_slot: u32) -> u32 {
    return probe_slot_table[chunk_slot];
}

/// Look up the chunk pool's *chunk* slot index for a chunk coordinate.
/// Returns `V3_PROBE_SENTINEL` if the chunk isn't in the slot table.
/// Mirrors `dda_chunk_to_slot` from dda_common.wgsl.
///
/// Requires `slot_table` and `slot_table_params` bindings. The consumer
/// declares them at whatever group/binding it likes.
fn v3_chunk_to_chunk_slot(chunk: vec3i) -> u32 {
    let origin = slot_table_params.xyz;
    let dim = slot_table_params.w;
    let local = chunk - origin;
    if (any(local < vec3i(0)) || any(local >= vec3i(dim))) {
        return V3_PROBE_SENTINEL;
    }
    let idx = u32(local.x)
            + u32(local.y) * u32(dim)
            + u32(local.z) * u32(dim) * u32(dim);
    return slot_table[idx];
}

// ─── Chunk → world / probe coordinate mapping ───────────────────────────
//
// `cascade_common.wgsl` is self-contained: it does *not* depend on
// `dda_common.wgsl`. The chunk-origin computation is inlined here so this
// file can be prepended to either the build pass (which also uses
// dda_common) or the consumer pass (which doesn't need full DDA traversal).
//
// CS is the chunk stride in voxels (62 for the project's 64³ padded
// chunks). It must match `pool::CS`.
const V3_CHUNK_STRIDE: i32 = 62;

/// World voxel coordinate of local (0,0,0) inside `chunk`. Mirrors
/// `dda_chunk_world_origin` from dda_common.wgsl: `chunk * 62 - 1`.
fn v3_chunk_world_origin(chunk: vec3i) -> vec3f {
    return vec3f(chunk) * f32(V3_CHUNK_STRIDE) - vec3f(1.0);
}

/// World-space voxel position of probe `(local_probe)` inside `chunk`.
///
/// Probes live on the padded 64³ grid at integer multiples of the cascade-0
/// spacing. The chunk's world origin (local voxel 0) is at
/// `chunk * CS - 1`, so the probe at local index `local_probe` is at:
///   world_voxel = chunk * CS - 1 + local_probe * V3_CASCADE_0_SPACING
fn v3_probe_world_voxel(chunk: vec3i, local_probe: vec3u) -> vec3f {
    let chunk_origin = v3_chunk_world_origin(chunk);
    let local_offset = vec3f(local_probe) * f32(V3_CASCADE_0_SPACING);
    return chunk_origin + local_offset;
}

/// Floor-divide a world voxel position to find which chunk *owns* it for
/// material/probe lookup purposes.
///
/// The chunk-ownership convention is "the chunk whose usable interior
/// contains this voxel". Per `dda_common.wgsl` line 210, chunk 0's usable
/// interior is world voxels [0, 61], chunk 1's is [62, 123], etc. So:
///
///     chunk = floor(world_voxel / 62)
///
/// **NOT** the same as `dda_common::dda_world_to_chunk`, which uses
/// `floor((world + 1) / 62)`. The DDA version is wrong by one at the high
/// edge of every chunk (a world voxel at the chunk's usable-interior
/// maximum gets assigned to the *next* chunk). The DDA gets away with it
/// because it iterates over chunks during traversal — if the first
/// chunk-coord is wrong by one, the next step lands on the right chunk
/// and the traversal continues. The consumer (`solid.wgsl`) does a single
/// per-fragment lookup with no iteration, so the mapping must be exact.
///
/// Concrete example: the Cornell box ceiling voxel at world y=61 is
/// chunk 0's usable interior (local y=62). The DDA mapping returns
/// chunk Y=1 (which doesn't exist) and the consumer reads sentinel,
/// producing pitch-black ceiling GI. The corrected mapping returns
/// chunk Y=0 and the lookup succeeds.
fn v3_world_to_chunk(world_voxel: vec3f) -> vec3i {
    let cs = f32(V3_CHUNK_STRIDE);
    return vec3i(
        i32(floor(world_voxel.x / cs)),
        i32(floor(world_voxel.y / cs)),
        i32(floor(world_voxel.z / cs)),
    );
}
