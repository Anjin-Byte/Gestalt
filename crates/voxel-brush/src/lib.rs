//! Pure brush kernels for the sculpt/paint system (`docs/design/brush-editing/03`,
//! Stage C): a read-only [`Field`] and a stamp in, a deterministic list of
//! [`VoxelOp`]s out. No GPU, no tree mutation, no RNG — same inputs, same ops,
//! every time. The effectful stroke controller that applies the ops (and owns
//! the undo journal + GPU sync) lives in `voxel-web`.
//!
//! The governing insight ([03 §the-governing-insight]): occupancy is binary, so
//! "soft" cannot mean half-set voxels. Hard deterministic stamps mark
//! (Draw/Erase/Clay), surface-relative kernels shape (Smooth/Flatten/Inflate),
//! and falloff/pressure shine where the domain is continuous (Paint's alpha,
//! smoothing intensity).
//!
//! [03 §the-governing-insight]: https://../docs/design/brush-editing/03-brush-engine.md

// Same numeric-cast posture as voxel-web/voxel-camera: brush geometry converts
// between voxel integers and floats constantly (≤ 2048³ grids, radius ≤ 12 —
// exact in f64; sign-loss sites are guarded non-negative before casting).
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

mod falloff;
mod kernels;
mod normal;
mod params;
mod stroke;

pub use falloff::weight;
pub use kernels::{Field, VoxelOp, stamp_ops};
pub use normal::estimate_normal;
pub use params::{BrushParams, BrushTool, Falloff, MAX_BRUSH_RADIUS};
pub use stroke::{Plane, Stamp, StrokeMask, mirrored, resample};
