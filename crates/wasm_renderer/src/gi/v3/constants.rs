//! v3 radiance cascades — Phase A constants.
//!
//! Single source of truth for cascade dimensions, probe counts, and storage
//! sizes. Mirrored in `cascade_common.wgsl` (manually kept in sync — Phase A
//! has no cross-target codegen).
//!
//! See `docs/Resident Representation/radiance-cascades-v3-design.md` §6 for
//! the locked decisions that drive these numbers.

use crate::pool;

/// Number of cascade levels active in Phase A. Phase B raises this to 6.
pub const V3_N_CASCADES: u32 = 1;

/// Cascade-0 probe spacing in voxel units. Probes live on a regular grid
/// at `world_voxel * V3_CASCADE_0_SPACING`.
pub const V3_CASCADE_0_SPACING: u32 = 4;

/// Octahedral direction grid resolution per axis at cascade 0.
/// Total directions per probe = `V3_CASCADE_0_DIRS_PER_AXIS²`.
pub const V3_CASCADE_0_DIRS_PER_AXIS: u32 = 8;

/// Total directions stored per cascade-0 probe (full sphere).
pub const V3_CASCADE_0_DIRS: u32 = V3_CASCADE_0_DIRS_PER_AXIS * V3_CASCADE_0_DIRS_PER_AXIS;

/// Maximum number of chunks that can simultaneously hold v3 probe payload.
/// Phase A budget cap — much smaller than `pool::MAX_SLOTS` because the
/// per-slot payload is large (2 MB). Cornell box needs only 1 slot.
pub const V3_MAX_PROBE_SLOTS: u32 = 64;

/// Sentinel value used in slot tables to indicate "no probe slot allocated
/// for this chunk". Same convention as `pool::SLOT_TABLE_SENTINEL`.
pub const V3_PROBE_SENTINEL: u32 = 0xFFFF_FFFF;

/// Probes per axis within a single chunk. Chunks are 64³ padded
/// (`pool::CS_P`); cascade-0 spacing is 4 voxels, so each chunk holds
/// `64 / 4 = 16` probes per axis.
///
/// Note: this uses `CS_P` (the padded dimension), not `CS` (usable),
/// because probes are stored on the full padded grid for symmetry with
/// occupancy data. The padding probes overlap neighboring chunks; their
/// values are valid because the build pass casts rays from world-space
/// positions, not from chunk-local positions.
pub const V3_PROBES_PER_CHUNK_AXIS: u32 = pool::CS_P / V3_CASCADE_0_SPACING;

/// Total probes stored per chunk slot (`V3_PROBES_PER_CHUNK_AXIS³`).
pub const V3_PROBES_PER_CHUNK: u32 =
    V3_PROBES_PER_CHUNK_AXIS * V3_PROBES_PER_CHUNK_AXIS * V3_PROBES_PER_CHUNK_AXIS;

/// Bytes per direction texel.
///
/// Phase A stores probe radiance as `vec4<f32>` (16 bytes) because WebGPU
/// storage buffer access for `f16` requires the `shader-f16` feature
/// extension that isn't enabled in this renderer. Phase B may switch to
/// packed f16 once the extension is gated on.
///
/// IMPORTANT: this stride must match the binding type the shader uses for
/// `probe_payload`. Currently `array<vec4<f32>>` in `cascade_build.wgsl`
/// and (eventually) in `solid.wgsl`.
pub const V3_PROBE_PAYLOAD_BYTES_PER_DIR: u32 = 16;

/// Bytes per probe (all directions stored consecutively).
pub const V3_PROBE_PAYLOAD_BYTES_PER_PROBE: u32 =
    V3_CASCADE_0_DIRS * V3_PROBE_PAYLOAD_BYTES_PER_DIR;

/// Bytes per chunk slot (all probes stored consecutively).
pub const V3_PROBE_PAYLOAD_BYTES_PER_SLOT: u32 =
    V3_PROBES_PER_CHUNK * V3_PROBE_PAYLOAD_BYTES_PER_PROBE;

/// Total payload SSBO size in bytes (`V3_MAX_PROBE_SLOTS` × per-slot).
/// Phase A: 64 × 2 MB = 128 MB.
pub const V3_PROBE_PAYLOAD_BUF_BYTES: u64 =
    V3_MAX_PROBE_SLOTS as u64 * V3_PROBE_PAYLOAD_BYTES_PER_SLOT as u64;

// ─── Compile-time invariants ────────────────────────────────────────────

const _: () = assert!(
    pool::CS_P % V3_CASCADE_0_SPACING == 0,
    "V3_CASCADE_0_SPACING must evenly divide CS_P (chunk padded dim)",
);

const _: () = assert!(
    V3_PROBES_PER_CHUNK == 4096,
    "Phase A expects 16³ = 4096 probes per chunk (CS_P=64, spacing=4)",
);

const _: () = assert!(
    V3_CASCADE_0_DIRS == 64,
    "Phase A expects 8×8 = 64 directions per probe",
);

const _: () = assert!(
    V3_PROBE_PAYLOAD_BYTES_PER_SLOT == 4 * 1024 * 1024,
    "Phase A per-slot payload must be exactly 4 MB (vec4f stride)",
);

const _: () = assert!(
    V3_PROBE_PAYLOAD_BUF_BYTES == 256 * 1024 * 1024,
    "Phase A total payload buffer must be exactly 256 MB",
);
