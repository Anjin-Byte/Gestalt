//! v3 cascade GPU resources — Phase A.
//!
//! Owns the SSBOs and bind group layout that the v3 build pass writes to
//! and that `solid.wgsl` will eventually read from. Phase A allocates only
//! the cascade-0 payload; Phase B will extend with additional per-cascade
//! buffers.
//!
//! Layout overview (Phase A):
//!
//! - **`probe_payload_buf`** — `V3_PROBE_PAYLOAD_BUF_BYTES` (256 MB).
//!   Storage buffer holding `vec4<f32>` direction texels for every probe
//!   in every allocated chunk slot. Layout is slot-major / probe-major /
//!   dir-minor; see `gi::v3::reference::flat_payload_byte_offset`.
//!   Phase A uses f32 because WebGPU `shader-f16` is not gated on; Phase B
//!   may switch to packed f16 to halve this to 128 MB.
//! - **`probe_slot_table_buf`** — 4 bytes × `pool::MAX_SLOTS` (16 KB).
//!   Storage buffer indexed by *chunk* slot (the chunk pool's slot, not
//!   the v3 probe slot). Each entry is the v3 probe slot for that chunk,
//!   or `V3_PROBE_SENTINEL` for chunks without a v3 allocation.
//! - **`cascade_params_buf`** — uniform buffer holding the v3 grid
//!   parameters and frame index. Updated each frame from the CPU.
//!
//! The bind group layout (`group3_layout`) is FRAGMENT-visible because it
//! is consumed by `solid.wgsl` for shading. The v3 build pass will create
//! its own COMPUTE-visible layout in `gi::v3::dispatch` (Phase A Commit 4)
//! to write `probe_payload_buf` as `read_write`.

use crate::gi::v3::constants::{V3_MAX_PROBE_SLOTS, V3_PROBE_PAYLOAD_BUF_BYTES};
use crate::pool;

/// CPU representation of the v3 cascade params uniform. Mirrors the WGSL
/// `V3CascadeParams` struct in `cascade_common.wgsl` (added in Commit 4).
///
/// Layout: 64 bytes total, naturally 16-byte aligned for WebGPU UBO.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Debug)]
pub struct V3CascadeParams {
    /// Scene grid origin in world space (matches `pool::ChunkPool::scene_params_buf`).
    pub grid_origin: [f32; 3],   // offset 0
    /// World units per voxel (typically 1.0).
    pub voxel_scale: f32,        // offset 12
    /// `V3_CASCADE_0_SPACING`. Stored in the uniform so the shader doesn't
    /// hardcode it (lets us tweak from CPU without recompiling shaders).
    pub cascade_0_spacing: u32,  // offset 16
    /// `V3_CASCADE_0_DIRS_PER_AXIS`.
    pub dirs_per_axis: u32,      // offset 20
    /// `V3_PROBES_PER_CHUNK_AXIS`.
    pub probes_per_chunk_axis: u32, // offset 24
    /// `V3_MAX_PROBE_SLOTS`. Used by the build pass for bounds checks.
    pub max_probe_slots: u32,    // offset 28
    /// Frame counter, incremented each frame. Phase C will use this for
    /// temporal accumulation; Phase A leaves it for sanity checking.
    pub frame_index: u32,        // offset 32
    /// Number of v3 probe slots that are currently in use. The build pass
    /// dispatches one workgroup-set per resident slot; this tells it where
    /// to stop iterating.
    pub active_probe_slots: u32, // offset 36
    /// Padding to bring total to 64 bytes (16-aligned).
    pub _pad: [u32; 6],          // offset 40-63
}

const _: () = assert!(
    std::mem::size_of::<V3CascadeParams>() == 64,
    "V3CascadeParams must be exactly 64 bytes",
);

/// All v3 cascade GPU resources owned by the renderer.
///
/// Allocated unconditionally at startup. The Phase A `probe_payload_buf`
/// reserves 128 MB even when `GI_PIPELINE = V2Legacy` — the alternative
/// (lazy allocation on selector flip) would add complexity for marginal
/// memory savings during development.
#[cfg(target_arch = "wasm32")]
pub struct V3CascadeResources {
    /// Probe radiance payload SSBO (rgba16f, slot-major).
    pub probe_payload_buf: wgpu::Buffer,
    /// Maps `chunk_slot → probe_slot` (or sentinel). Indexed by
    /// `pool::MAX_SLOTS` chunk slot indices.
    pub probe_slot_table_buf: wgpu::Buffer,
    /// Compact reverse lookup: `v3_probe_slot → (chunk_x, chunk_y, chunk_z, chunk_slot)`
    /// packed as `vec4<i32>`. Length `V3_MAX_PROBE_SLOTS`. Used by the
    /// build pass to convert a probe slot index back into the chunk it
    /// belongs to in O(1) instead of scanning the slot table.
    /// CPU writes this each time the slot table is updated.
    pub active_probe_chunks_buf: wgpu::Buffer,
    /// Uniform buffer holding `V3CascadeParams`. Written each frame.
    pub cascade_params_buf: wgpu::Buffer,
    /// FRAGMENT-visible bind group layout for the v3 group 3 used by
    /// `solid.wgsl`. Wired into the color pipeline in Phase A Commit 5.
    pub group3_layout: wgpu::BindGroupLayout,
}

#[cfg(target_arch = "wasm32")]
impl V3CascadeResources {
    /// Allocate all v3 GPU resources. Called once during `Renderer::new`.
    pub fn new(device: &wgpu::Device) -> Self {
        // ── Probe payload SSBO (128 MB) ─────────────────────────────
        let probe_payload_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("v3-probe-payload"),
            size: V3_PROBE_PAYLOAD_BUF_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Probe slot table (chunk_slot → probe_slot) ──────────────
        // 4 bytes per chunk slot; sentinel-filled at startup.
        let probe_slot_table_size =
            (pool::MAX_SLOTS as u64) * (std::mem::size_of::<u32>() as u64);
        let probe_slot_table_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("v3-probe-slot-table"),
            size: probe_slot_table_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Active probe → chunk lookup ──────────────────────────────
        // 16 bytes (vec4<i32>) per active probe slot. 64 entries = 1 KB.
        let active_probe_chunks_size =
            (V3_MAX_PROBE_SLOTS as u64) * (std::mem::size_of::<[i32; 4]>() as u64);
        let active_probe_chunks_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("v3-active-probe-chunks"),
            size: active_probe_chunks_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Cascade params uniform (64 bytes) ───────────────────────
        let cascade_params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("v3-cascade-params"),
            size: std::mem::size_of::<V3CascadeParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Group 3 bind group layout (FRAGMENT consumer) ───────────
        // Five bindings, all read-only from the fragment stage.
        //
        // Bindings 0-1 are the chunk pool's *chunk* slot table — needed
        // by the consumer to find the chunk a world position belongs to,
        // before looking up the v3 probe slot for that chunk.
        // Bindings 2-4 are v3's probe data.
        let group3_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("v3-cascade-group3-layout"),
            entries: &[
                // @binding(0) chunk_slot_table (chunk pool) — read storage
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // @binding(1) chunk_slot_table_params (chunk pool) — uniform
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // @binding(2) probe_payload — read-only storage
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // @binding(3) probe_slot_table — read-only storage
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // @binding(4) cascade_params — uniform
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Initialize the slot table to all-sentinel so unallocated chunks
        // read as "no probe data". The renderer will overwrite entries as
        // chunks are allocated (Phase A Commit 3).
        // Note: we don't have a queue here, so the actual sentinel fill
        // happens in `Renderer::new` after the queue is available, via
        // `Self::clear_slot_table`.
        Self {
            probe_payload_buf,
            probe_slot_table_buf,
            active_probe_chunks_buf,
            cascade_params_buf,
            group3_layout,
        }
    }

    /// Build the consumer-side group 3 bind group used by `solid.wgsl`.
    /// Combines chunk-pool slot table buffers (passed in by reference) with
    /// the v3 probe data this struct owns.
    pub fn create_group3_bind_group(
        &self,
        device: &wgpu::Device,
        chunk_slot_table_buf: &wgpu::Buffer,
        chunk_slot_table_params_buf: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("v3-cascade-group3-bg"),
            layout: &self.group3_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: chunk_slot_table_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: chunk_slot_table_params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.probe_payload_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.probe_slot_table_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.cascade_params_buf.as_entire_binding(),
                },
            ],
        })
    }

    /// Reset the probe slot table to all-sentinel and the active probe
    /// chunks list to zero. Call once after creation (the queue isn't
    /// available inside `new`) and on scene clear.
    pub fn clear_slot_table(&self, queue: &wgpu::Queue) {
        let sentinel_table: Vec<u32> =
            vec![crate::gi::v3::constants::V3_PROBE_SENTINEL; pool::MAX_SLOTS as usize];
        queue.write_buffer(
            &self.probe_slot_table_buf,
            0,
            bytemuck::cast_slice(&sentinel_table),
        );
        // Sentinel: w = -1 means "this v3 probe slot is unallocated".
        // Kernel uses this for early-out without consulting the slot table.
        let sentinel: Vec<[i32; 4]> = vec![[0, 0, 0, -1]; V3_MAX_PROBE_SLOTS as usize];
        queue.write_buffer(
            &self.active_probe_chunks_buf,
            0,
            bytemuck::cast_slice(&sentinel),
        );
    }
}
