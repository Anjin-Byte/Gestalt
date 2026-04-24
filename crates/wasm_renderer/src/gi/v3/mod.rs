//! Radiance cascades v3 — world-space probes, sparse chunk-keyed allocation.
//!
//! Architecture: `docs/Resident Representation/radiance-cascades-v3-design.md`.
//! v2 failure modes that motivate the rewrite:
//! `docs/Resident Representation/radiance-cascades-symptoms.md`.
//!
//! ## Phase A deliverables (in progress)
//! - `cascade_probe_alloc` — chunk-keyed slot table for probe payload
//! - `cascade_residency`   — distance-based per-cascade residency rules
//! - `cascade_build`       — per-frame world-space cascade build dispatch
//! - `shaders/cascade_common.wgsl` — probe addressing, octahedral, trilinear
//! - `shaders/cascade_build.wgsl`  — hemisphere ray casting via DDA
//!
//! Phase A constructs cascade 0 only with no merge. The single-cascade
//! baseline must produce flicker-free, leak-free near-field GI in the
//! Cornell box test before Phase B (full cascade hierarchy + shade-time
//! merge) begins.

pub mod constants;
pub mod probe_slot;
pub mod reference;

#[cfg(target_arch = "wasm32")]
pub mod backend;
#[cfg(target_arch = "wasm32")]
pub mod dispatch;
#[cfg(target_arch = "wasm32")]
pub mod resources;
