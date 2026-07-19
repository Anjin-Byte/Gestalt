//! Pure camera math + input snapshot for the interactive front ends.
//!
//! Extracted from `voxel-viewer` (stage 2 of `docs/design/web-frontend-api.md`
//! §5) so the winit viewer and the web engine share one implementation. Two
//! cameras produce one [`GpuCamera`](voxel_core::GpuCamera) uniform:
//!
//! - [`orbit_camera`] — a deterministic turntable (scripted profiling runs and
//!   the default until the user takes control).
//! - [`FlyCamera`] — an interactive free-fly camera driven by [`Input`].
//!
//! Everything here is pure: positions and angles in, a uniform out. No
//! windowing, no device access, no clock — each front end's event loop reads
//! its own clock and devices and feeds the results in as `dt` and an [`Input`]
//! snapshot (Engineering Codex: *Pure Core, Effectful Edges*). That keeps the
//! math unit-testable with zero setup.

// Same numeric-cast posture as the front ends this was extracted from:
// viewport and grid dimensions (≤ 2048³ grids, small viewports — exact in
// f32) convert between integer and float constantly in camera math.
#![allow(clippy::cast_precision_loss)]

mod camera;
mod input;

pub use camera::{
    FlyCamera, OrbitControl, OrbitFrame, fit_orbit_radius, orbit_camera, orbit_eye_forward,
};
pub use input::Input;
