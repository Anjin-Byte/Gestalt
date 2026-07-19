//! Web front end: the WASM kernel behind the `Engine` boundary.
//!
//! This cdylib is consumed by the TypeScript shell via wasm-pack. The boundary
//! is designed in `docs/design/web-frontend-api.md` — TS owns the canvas element, the
//! `requestAnimationFrame` loop, DOM input, and the HUD; this crate owns the
//! WebGPU device, pipelines, structure, and camera, behind one substantial
//! boundary crossing per frame (`Engine::frame` — wasm32-only, so no doc
//! link resolves natively).
//!
//! The pure modules (camera, input, scene, blit) compile on every target so
//! their tests and lints run natively; the bindgen surface is `wasm32`-only
//! because `wgpu::SurfaceTarget::Canvas` and `web-sys` exist only there.

// Same numeric-cast posture as voxel-viewer: viewport and grid dimensions
// (≤ 2048³ grids, small viewports — exact in f32) convert between integer and
// float constantly in camera/render code.
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

// On native the pure modules have no consumer (the wasm32-only `engine` is
// their only caller), hence the scoped dead-code allowance per module.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod blit;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod edit;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod mesh;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod phases;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod scene;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod scene_transfer;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod undo;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod vox;

#[cfg(target_arch = "wasm32")]
mod engine;
#[cfg(target_arch = "wasm32")]
mod iokernel;

// The tool/falloff enums live in the pure voxel-brush crate (Stage C) and
// cross the boundary from there; re-exported so the generated JS surface is
// unchanged.
#[cfg(target_arch = "wasm32")]
pub use engine::{CameraMode, Engine, FrameStats, KeyAction, SceneInfo};
#[cfg(target_arch = "wasm32")]
pub use iokernel::{IoKernel, MeshFormat};
#[cfg(target_arch = "wasm32")]
pub use voxel_brush::{BrushTool, Falloff};
