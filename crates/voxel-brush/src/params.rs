//! Brush configuration: the tool and falloff enums (which cross the wasm
//! boundary as C-like enums — the `CameraMode`/`KeyAction` precedent) and the
//! parameter block the control plane sets.

// The enums cross the wasm boundary via wasm-bindgen, but this crate also
// compiles natively for its tests (where wasm-bindgen is not a dependency), so
// the attribute is applied only on wasm32; natively they are plain C-like enums.
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;

/// Largest edit-brush radius (voxels); a radius-`r` sphere is `~(2r+1)³`
/// voxels (matches the native viewer's cap).
pub const MAX_BRUSH_RADIUS: u32 = 12;

/// The seven brush tools (`docs/design/brush-editing/03`). Exposed across the
/// wasm boundary so the shell's tool palette selects one. Variant order is
/// append-only: the A3 trio keeps its discriminants, the Stage-C sculpt set
/// follows (display order in the HUD is the design's, independent of this).
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
// `pub` for wasm-bindgen; natively consumed only through voxel-web's tests.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BrushTool {
    /// Add occupancy: every empty voxel in the ball sets, carrying the brush
    /// colour on a truecolor scene. Occupied voxels are untouched (recolouring
    /// is Paint's job). The reliable blockout tool — deliberately no falloff.
    Draw,
    /// Remove occupancy (and its colour). Draw's inverse; no falloff.
    Erase,
    /// Blend the brush colour onto occupied voxels (truecolor scenes only);
    /// falloff × strength × pressure is the per-voxel alpha, max-combined
    /// through the per-stroke mask.
    Paint,
    /// Surface-biased buildup: empty voxels touching the surface set where
    /// falloff × strength ≥ ½, inheriting the average neighbour colour —
    /// material accretes like clay coils instead of detached spheres.
    Clay,
    /// The feel brush: a 3³ density filter re-thresholded at ½ (two passes per
    /// stamp) — bumps erode, pits fill; colours box-filter along.
    Smooth,
    /// Slab clamp against a plane anchored at stroke start: material above the
    /// plane cuts, supported gaps just below it fill — dragging produces flat
    /// facets instead of chasing the local surface.
    Flatten,
    /// Uniform dilation of the covered surface: empty surface-adjacent voxels
    /// within `strength × radius` of the centre set — fattens forms evenly
    /// (Clay without the centre bias).
    Inflate,
}

/// The brush falloff curve over `t ∈ [0, 1]` (0 at the centre, 1 at the rim).
/// All satisfy `w(0) = 1`, `w(1) = 0`, monotone non-increasing. Drives Paint's
/// per-voxel alpha, Clay's buildup gate, and Smooth's intensity (the hard
/// tools ignore it).
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Falloff {
    /// Smoothstep shoulder (the default).
    Smooth,
    /// Even linear ramp.
    Linear,
    /// Full-strength core, fast edge drop.
    Sphere,
    /// Concentrated centre.
    Sharp,
}

/// The current brush configuration — set by the control plane (`set_brush`) and
/// read on every `brush` event. Scalars only; no per-event allocation.
#[derive(Clone, Copy)]
pub struct BrushParams {
    /// The active tool.
    pub tool: BrushTool,
    /// Radius in voxels (capped at [`MAX_BRUSH_RADIUS`]).
    pub radius: u32,
    /// Tool intensity in `[0, 1]` (Paint alpha, Clay/Smooth aggressiveness,
    /// Flatten slab depth, Inflate reach).
    pub strength: f32,
    /// The falloff curve over the ball.
    pub falloff: Falloff,
    /// The brush colour (sRGB RGBA8, R in the low byte).
    pub color: u32,
    /// The inverted arm of a tool (Alt held): Inflate becomes deflate —
    /// exposed surface voxels thin instead of growing. Other tools ignore it
    /// in v1.
    pub invert: bool,
}

impl Default for BrushParams {
    fn default() -> Self {
        Self {
            tool: BrushTool::Draw,
            radius: 3,
            strength: 1.0,
            falloff: Falloff::Smooth,
            color: 0xFFFF_FFFF, // opaque white
            invert: false,
        }
    }
}
