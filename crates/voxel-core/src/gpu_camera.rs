//! The camera uniform of the GPU render contract.
//!
//! Lives here with the rest of the buffer contract ([`GpuNode`], the School-B
//! word layouts) rather than in the wgpu adapter: every front end that builds
//! camera poses needs the struct, and none of them need wgpu for it.
//!
//! [`GpuNode`]: crate::GpuNode

// Unsafe Quarantine: the `bytemuck` derives on `#[repr(C)]` all-scalar data;
// none is hand-written (same posture as `node.rs`).
#![allow(unsafe_code)]

use bytemuck::{Pod, Zeroable};

/// Camera uniform, matching `render.wgsl`'s `Camera` (std140-friendly: every
/// `vec3` is followed by a scalar to fill its 16-byte slot).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuCamera {
    /// Camera world position.
    pub eye: [f32; 3],
    /// `tan(fov/2)`.
    pub tan: f32,
    /// Forward (unit) direction.
    pub forward: [f32; 3],
    /// Width / height.
    pub aspect: f32,
    /// Right (unit) direction.
    pub right: [f32; 3],
    /// Grid resolution `n` as `f32`.
    pub n: f32,
    /// Up (unit) direction.
    pub up: [f32; 3],
    /// Padding to keep the following `dims` 16-byte aligned.
    pub pad: f32,
    /// `[width, height, internal_levels(k), 0]`.
    pub dims: [u32; 4],
}
