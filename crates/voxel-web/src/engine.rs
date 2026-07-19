//! The `#[wasm_bindgen]` boundary surface: the `Engine` handle the TS shell
//! drives, plus the small typed values that cross with it.
//!
//! The boundary discipline is `docs/design/web-frontend-api.md` §4: control-plane
//! calls (`create`, `load_fixture`, `voxelize_mesh`, `resize`) are rare and may
//! carry object shapes via `serde-wasm-bindgen`; data-plane calls (`frame`,
//! input setters, `stats`) are per-frame/event-rate and cross scalars only.
//! Any change to a `pub` item here changes the generated `.d.ts` — update the
//! design doc alongside (§9).

use voxel_core::{MaterialTable, Resolution, SchoolBBuffer, SparseTree};
use voxel_gpu::{GpuCamera, GpuContext, GpuRenderer};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;
use wgpu::CurrentSurfaceTexture::{Suboptimal, Success};

use voxel_camera::{FlyCamera, Input, OrbitControl, OrbitFrame, fit_orbit_radius};

use voxel_brush::{BrushParams, BrushTool, Falloff, MAX_BRUSH_RADIUS, Stamp};

use crate::edit::{self, StrokeState};
use crate::undo::UndoRing;
use crate::{blit, scene, scene_transfer};

/// Upper clamp on a frame's `dt`, seconds. `requestAnimationFrame` pauses in
/// background tabs; an unclamped resume `dt` would teleport the fly camera.
const MAX_DT: f32 = 0.1;

/// The camera control scheme, chosen explicitly by the shell (HUD buttons).
/// Right-drag is the brush in every mode; the modes differ in what left-drag
/// and the wheel do.
#[wasm_bindgen]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CameraMode {
    /// The hybrid orbit (the initial state): a fresh scene spins ambiently
    /// until first grabbed; left-drag then rotates around the pivot with fling
    /// momentum on release, the wheel zooms, Alt-drag pans the pivot, and
    /// double-click recentres it.
    Orbit,
    /// Free-fly: left-drag looks, `WASD`/`QE` move, the wheel sets speed.
    Fly,
}

/// Control-plane options for [`Engine::create`], deserialized from a plain JS
/// object (`{ res?: number, fixture?: string }`).
#[derive(serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct EngineOptions {
    /// Grid resolution per axis (must be a legal `8·4^k` size).
    res: u32,
    /// Named CPU fixture to build initially (see [`scene::build_fixture`]).
    fixture: String,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            // 128 is the largest 8·4^k size whose single-threaded CPU fixture
            // build feels instant on the main thread.
            res: 128,
            fixture: "wire-lattice".to_string(),
        }
    }
}

/// Movement/modifier actions the shell maps DOM key events onto — the shell
/// owns the physical keymap; the kernel only sees intents.
#[wasm_bindgen]
#[derive(Clone, Copy)]
pub enum KeyAction {
    /// Move along the forward direction (`W` by convention).
    Forward,
    /// Move opposite the forward direction (`S`).
    Back,
    /// Strafe left (`A`).
    Left,
    /// Strafe right (`D`).
    Right,
    /// Rise along world `+Y` (`E` / Space).
    Up,
    /// Descend along world `−Y` (`Q` / Ctrl).
    Down,
    /// Movement speed boost while held (Shift).
    Boost,
}

/// What a scene build produced, returned by the control plane for the shell's
/// UI (scene picker, HUD header).
#[wasm_bindgen]
pub struct SceneInfo {
    /// Human-readable scene name (fixture name or mesh filename).
    #[wasm_bindgen(getter_with_clone)]
    pub label: String,
    /// Internal-node count of the built structure.
    pub nodes: u32,
    /// Leaf-brick count of the built structure.
    pub leaves: u32,
    /// Occupied-voxel count of the built structure.
    pub voxels: u32,
    /// Grid resolution per axis.
    pub res: u32,
    /// Whether the scene takes brush edits. True for every scene since Stage A3
    /// (the truecolor build-once gate fell); the shell enables the brush on it.
    pub editable: bool,
    /// Whether the scene is truecolor (per-voxel editable colour). The shell
    /// gates the Paint tool on this — palette scenes can't paint until the
    /// promotion path (Stage D); Draw/Erase work on every scene.
    pub truecolor: bool,
}

/// Per-frame numbers for the shell's DOM HUD, read once per frame via
/// [`Engine::stats`]. Plain `Copy` fields — no serde on the hot path.
#[wasm_bindgen]
#[derive(Clone, Copy)]
pub struct FrameStats {
    /// Frames rendered since the engine (or current scene) was created.
    pub frames: u32,
    /// The `dt` the last [`Engine::frame`] call received, milliseconds.
    pub last_dt_ms: f64,
    /// Internal-node count of the current structure.
    pub nodes: u32,
    /// Leaf-brick count of the current structure.
    pub leaves: u32,
    /// Occupied-voxel count of the current structure (live during edits).
    pub voxels: u32,
    /// Grid resolution per axis.
    pub res: u32,
    /// Strokes available to undo (0 = the undo button disables). Drives the
    /// HUD's history buttons + depth display at the stats cadence.
    pub undo_depth: u32,
    /// Strokes available to redo (0 = the redo button disables).
    pub redo_depth: u32,
    /// Whether the scene carries per-voxel colour — flips mid-session when the
    /// first Paint stroke promotes a palette scene (Stage D); the shell watches
    /// it to confirm the promotion in the status line.
    pub truecolor: bool,
}

/// The WASM kernel handle: owns the WebGPU device, swapchain, pipelines,
/// structure, and camera state. The TS shell owns the canvas element, the
/// `requestAnimationFrame` loop, and all DOM input listeners.
#[wasm_bindgen]
pub struct Engine {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    ctx: GpuContext,
    /// `None` only mid-[`install_scene`] (the old renderer is dropped before
    /// the new scene's buffers are created, so GPU residency peaks at one
    /// scene, not two) — and after a failed install, where frames skip until
    /// the next install succeeds.
    renderer: Option<GpuRenderer>,
    output_view: wgpu::TextureView,
    blit_pipeline: wgpu::RenderPipeline,
    blit_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    blit_bind: wgpu::BindGroup,
    resolution: Resolution,
    scene_label: String,
    // The live CPU-side scene, kept for voxel-native export (and, later,
    // edits): the tree, its School-B buffer, and the palette table.
    tree: SparseTree,
    structure: SchoolBBuffer,
    table: MaterialTable,
    nodes: u32,
    leaves: u32,
    voxels: u32,
    mode: CameraMode,
    /// The hybrid-orbit state (azimuth/elevation/radius + pivot offset and
    /// momentum), driven by drag + wheel in `Orbit` mode and integrated each
    /// frame. Scale-invariant, so it survives a re-derivation unchanged.
    orbit: OrbitControl,
    /// What the orbit modes pivot on and frame — the current scene's occupied
    /// bounding box (or the whole grid when the scene is empty). Recomputed on
    /// every scene change so a mesh in a corner of a large grid is centred on
    /// itself, not on the empty grid.
    orbit_frame: OrbitFrame,
    fly: FlyCamera,
    input: Input,
    /// The camera of the most recent frame, kept so a brush click can re-cast
    /// the exact on-screen ray.
    last_camera: GpuCamera,
    /// The current brush configuration (tool/radius/strength/falloff/colour),
    /// set by the control-plane [`set_brush`](Self::set_brush) and read on every
    /// [`brush`](Self::brush) event.
    brush_params: BrushParams,
    /// The previous pointer event's stamp within the active stroke — the
    /// anchor `voxel_brush::resample` interpolates from (centre + pressure).
    /// `None` between strokes (and after a miss, which honestly breaks the
    /// stroke).
    stroke_last: Option<Stamp>,
    /// Per-stroke mutable state: the paint max-alpha mask (cleared on
    /// `brush_end`) and the undo journal (committed into the ring there).
    stroke: StrokeState,
    /// The surface point (voxel centre) under the unpressed pointer, if any —
    /// drives the GPU hover-cursor ring, uploaded each frame.
    hover: Option<[f32; 3]>,
    /// The themed sky endpoints (top, bottom — sRGB RGBA8, R low), kept so a
    /// renderer recreation (install, promotion) re-applies the theme.
    sky: Option<(u32, u32)>,
    /// Sun-shadow quality (0 off, 1 low = coarse brick trace, 2 high = exact
    /// per-voxel trace) — web default OFF (a real per-pixel cost); the shell's
    /// settings control re-applies it across renderer rebuilds.
    shadow_quality: u32,
    /// Whether the GTAO term runs at all (off skips the AO + denoise passes).
    gtao_on: bool,
    /// GTAO quality preset index (0 Low, 1 Medium, 2 High, 3 Ultra).
    gtao_preset: u32,
    /// The per-stroke undo/redo history (`docs/design/brush-editing/05`).
    undo_ring: UndoRing,
    /// Set when a topology re-upload failed: the renderer's buffers are stale,
    /// so the next edit must re-upload fully rather than patch into them.
    needs_full_upload: bool,
    frames: u32,
    last_dt_ms: f64,
}

/// Applies a shell shadow-quality level to a renderer's shadow knobs:
/// 0 off; 1 low = the EXACT trace at ½×½ resolution, joint-bilateral
/// upsampled (~4× fewer rays, correct-shaped umbras); 2 high = the exact
/// full-resolution trace.
fn apply_shadow_quality(renderer: &mut GpuRenderer, quality: u32) {
    renderer.set_shadows(quality > 0);
    renderer.set_half_res_shadows(quality == 1);
}

/// The renderer-side GTAO params for a shell preset index (0 Low … 3 Ultra).
/// Mirrors the native viewer's G-cycle table; radius matches the default.
fn gtao_params_for(preset: u32) -> voxel_gpu::GtaoParams {
    let (slice_count, steps_per_slice) = match preset {
        0 => (1.0, 2.0),
        2 => (3.0, 3.0),
        3 => (9.0, 3.0),
        _ => (2.0, 2.0), // 1 = Medium (the renderer default)
    };
    voxel_gpu::GtaoParams {
        slice_count,
        steps_per_slice,
        ..voxel_gpu::GtaoParams::default()
    }
}

#[wasm_bindgen]
impl Engine {
    /// Async init: WebGPU adapter/device request plus a swapchain on `canvas`
    /// (the one-time control-plane handoff of the canvas handle — TS keeps
    /// owning the element itself). `opts` is an optional
    /// `{ res?: number, fixture?: string }` object.
    ///
    /// Rejects when WebGPU is unavailable — there is no WebGL fallback by
    /// construction (`docs/design/web-frontend-api.md` §8).
    pub async fn create(canvas: HtmlCanvasElement, opts: JsValue) -> Result<Engine, JsError> {
        let (width, height) = (canvas.width().max(1), canvas.height().max(1));
        Self::create_with_target(wgpu::SurfaceTarget::Canvas(canvas), width, height, opts).await
    }

    /// [`create`](Self::create) for a render worker: the swapchain target is an
    /// `OffscreenCanvas` (main transfers control of its element and never
    /// touches the backing store again — stage 7 phase 2). Everything else is
    /// identical; the shell falls back to `create` on the main thread where
    /// worker rendering is unavailable.
    pub async fn create_offscreen(
        canvas: web_sys::OffscreenCanvas,
        opts: JsValue,
    ) -> Result<Engine, JsError> {
        let (width, height) = (canvas.width().max(1), canvas.height().max(1));
        Self::create_with_target(
            wgpu::SurfaceTarget::OffscreenCanvas(canvas),
            width,
            height,
            opts,
        )
        .await
    }

    async fn create_with_target(
        target: wgpu::SurfaceTarget<'static>,
        width: u32,
        height: u32,
        opts: JsValue,
    ) -> Result<Engine, JsError> {
        console_error_panic_hook::set_once();
        let opts: EngineOptions = if opts.is_undefined() || opts.is_null() {
            EngineOptions::default()
        } else {
            serde_wasm_bindgen::from_value(opts).map_err(|e| JsError::new(&e.to_string()))?
        };

        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(target)
            .map_err(|e| JsError::new(&format!("surface from canvas: {e}")))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|e| JsError::new(&format!("no WebGPU adapter: {e}")))?;

        // Ask for as much storage-buffer headroom as the browser grants — the
        // renderer's chunk probe splits or fails loudly against these caps.
        let adapter_limits = adapter.limits();
        let limits = wgpu::Limits {
            max_storage_buffer_binding_size: adapter_limits.max_storage_buffer_binding_size,
            max_buffer_size: adapter_limits.max_buffer_size,
            ..wgpu::Limits::default()
        };
        // No TIMESTAMP_QUERY on the web path yet: its readback is blocking, and
        // kernel timing is the stage-5 latest-available design.
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("voxel-web device"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                memory_hints: wgpu::MemoryHints::Performance,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| JsError::new(&format!("request_device: {e}")))?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats[0];
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        let ctx = GpuContext { device, queue };
        surface.configure(&ctx.device, &config);

        let resolution = Resolution::new(opts.res).map_err(|e| JsError::new(&e.to_string()))?;
        // The initial scene builds inline on the render thread; no progress
        // channel here (the shell shows the bar for worker-side builds).
        let (tree, structure) = scene::build_fixture(&opts.fixture, resolution, &mut |_, _, _| {})
            .map_err(|e| JsError::new(&e.to_string()))?;
        // Fixtures carry no materials: the magenta-only table makes every hit
        // global-0, which the shader shades by position.
        let table = MaterialTable::missing_only();
        let mut renderer =
            GpuRenderer::new(&ctx, &structure, &table).map_err(|e| JsError::new(&e.to_string()))?;
        // Web defaults: shadows off (perf), AO on, Medium GTAO.
        apply_shadow_quality(&mut renderer, 0);

        let (blit_pipeline, blit_layout, sampler) = blit::build_blit(&ctx.device, format);
        let output_view = blit::make_output(&ctx.device, width, height);
        let blit_bind = blit::make_blit_bind(&ctx.device, &blit_layout, &output_view, &sampler);

        let n = resolution.voxels_per_axis() as f32;
        let bbox = tree.occupied_bbox();
        let orbit_frame = OrbitFrame::for_bbox(bbox, n);
        let aspect = config.width as f32 / config.height.max(1) as f32;
        let orbit = OrbitControl::default().with_radius(fit_orbit_radius(bbox, aspect));
        let (eye, fwd) = orbit.eye_forward(orbit_frame);
        Ok(Engine {
            surface,
            config,
            ctx,
            renderer: Some(renderer),
            output_view,
            blit_pipeline,
            blit_layout,
            sampler,
            blit_bind,
            resolution,
            scene_label: opts.fixture,
            nodes: saturating_u32(tree.node_count() as u64),
            leaves: saturating_u32(tree.leaf_count() as u64),
            voxels: saturating_u32(tree.occupied_voxels()),
            tree,
            structure,
            table,
            mode: CameraMode::Orbit,
            orbit,
            orbit_frame,
            fly: FlyCamera::from_eye_forward(eye, fwd, n),
            input: Input::default(),
            last_camera: orbit.to_gpu(orbit_frame, width, height, n, resolution.internal_levels()),
            brush_params: BrushParams::default(),
            shadow_quality: 0, // web default: shadows off (perf)
            gtao_on: true,
            gtao_preset: 1, // Medium

            stroke_last: None,
            stroke: StrokeState::default(),
            hover: None,
            sky: None,
            undo_ring: UndoRing::default(),
            needs_full_upload: false,
            frames: 0,
            last_dt_ms: 0.0,
        })
    }

    /// THE per-frame call (one substantial boundary crossing per frame):
    /// integrates accumulated input into the camera, encodes the render compute
    /// pass plus the blit, and submits. `dt_ms` comes from the shell's
    /// `performance.now()` deltas — the kernel never reads a clock.
    ///
    /// Never blocks on the GPU: WebGPU has no synchronous wait, and none is
    /// needed — presentation happens when control returns to the browser.
    pub fn frame(&mut self, dt_ms: f64) {
        let dt = (dt_ms.max(0.0) / 1000.0).min(f64::from(MAX_DT)) as f32;
        let camera = self.update_camera(dt);
        self.last_camera = camera;
        self.input.end_frame();

        let Some(renderer) = &mut self.renderer else {
            return; // mid-install / failed install: keep the last presented image
        };
        // The hover-cursor ring rides the per-frame upload (one 32-byte
        // uniform write; inactive = the byte-identical default).
        match self.hover {
            #[allow(clippy::cast_precision_loss)] // radius ≤ 12
            Some(pos) => renderer.set_cursor(pos, self.brush_params.radius as f32, true),
            None => renderer.set_cursor([0.0; 3], 0.0, false),
        }
        let (Success(frame) | Suboptimal(frame)) = self.surface.get_current_texture() else {
            // Lost/outdated swapchain: reconfigure and skip this frame.
            self.surface.configure(&self.ctx.device, &self.config);
            return;
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        // Compute: traverse + shade → output texture (untimed: kernel timing on
        // the web is the stage-5 latest-available design).
        renderer.render(
            &mut encoder,
            &camera,
            &self.output_view,
            self.config.width,
            self.config.height,
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.blit_pipeline);
            pass.set_bind_group(0, &self.blit_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.ctx.queue.submit([encoder.finish()]);
        frame.present();

        self.frames = self.frames.saturating_add(1);
        self.last_dt_ms = dt_ms;
    }

    /// Resizes the swapchain and the render-output texture. The shell calls
    /// this from its canvas-resize observer.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.ctx.device, &self.config);
        self.output_view =
            blit::make_output(&self.ctx.device, self.config.width, self.config.height);
        self.blit_bind = blit::make_blit_bind(
            &self.ctx.device,
            &self.blit_layout,
            &self.output_view,
            &self.sampler,
        );
    }

    /// Selects the camera control scheme (the shell's HUD buttons). Switching
    /// *into* a mode seeds it from the current on-screen pose so the view never
    /// jumps.
    pub fn set_camera_mode(&mut self, mode: CameraMode) {
        if self.mode == mode {
            return;
        }
        let eye = glam::Vec3::from_array(self.last_camera.eye);
        let forward = glam::Vec3::from_array(self.last_camera.forward);
        match mode {
            // Seed the orbit from the current eye so azimuth/elevation/radius
            // continue the view (settled: no ambient spin, no momentum).
            CameraMode::Orbit => self.orbit = OrbitControl::from_view(eye, self.orbit_frame),
            // Seed the fly camera from the current pose (same as the old
            // take-control handoff).
            CameraMode::Fly => {
                let n = self.resolution.voxels_per_axis() as f32;
                self.fly = FlyCamera::from_eye_forward(eye, forward, n);
            }
        }
        self.mode = mode;
    }

    /// Sets a held movement/modifier flag (fly-camera movement; inert in the
    /// orbit modes, which the flags never reach).
    pub fn key(&mut self, action: KeyAction, down: bool) {
        match action {
            KeyAction::Forward => self.input.forward = down,
            KeyAction::Back => self.input.back = down,
            KeyAction::Left => self.input.left = down,
            KeyAction::Right => self.input.right = down,
            KeyAction::Up => self.input.up = down,
            KeyAction::Down => self.input.down = down,
            KeyAction::Boost => self.input.boost = down,
        }
    }

    /// Applies a pointer-drag delta (pixels), routed by the active mode:
    /// `Orbit` rotates around the pivot (grabbing the ambient spin), `Fly`
    /// looks.
    pub fn pointer_delta(&mut self, dx: f32, dy: f32) {
        match self.mode {
            CameraMode::Orbit => self.orbit.drag(dx, dy),
            CameraMode::Fly => {
                self.input.look_dx += dx;
                self.input.look_dy += dy;
            }
        }
    }

    /// The look-drag pointer released: in `Orbit`, the pose's recent motion
    /// becomes the fling that friction glides to rest. Inert in `Fly`.
    pub fn look_end(&mut self) {
        if self.mode == CameraMode::Orbit {
            self.orbit.release();
        }
    }

    /// Pans the orbit pivot in the camera plane (Alt-drag, pixels): the focus
    /// can move to any point in space. Inert in `Fly` (which moves itself).
    pub fn pan(&mut self, dx: f32, dy: f32) {
        if self.mode == CameraMode::Orbit {
            self.orbit.pan(dx, dy, self.orbit_frame);
        }
    }

    /// Recentres the orbit pivot on the scene's own frame (double-click).
    pub fn reset_pivot(&mut self) {
        self.orbit.reset_pivot();
    }

    /// Applies scroll-wheel notches, routed by mode: `Orbit` zooms (dollies the
    /// radius), `Fly` adjusts movement speed.
    pub fn wheel(&mut self, notches: f32) {
        match self.mode {
            CameraMode::Orbit => self.orbit.zoom(notches),
            CameraMode::Fly => self.input.scroll += notches,
        }
    }

    /// Installs a scene blob built by the IO worker's `IoKernel`
    /// (`docs/design/web-frontend-api.md` §5, stage 7): deserializes,
    /// uploads to the renderer, and retains the CPU-side scene. Synchronous and
    /// fast (upload + pipeline bind) — the heavy building happened off-thread.
    ///
    /// `preserve_camera` keeps the current view: a *re-derivation* of the scene
    /// (a resolution change, a mesh re-voxelize) should not throw the user back
    /// to the default orbit. The shell decides — it alone knows whether a build
    /// is the same scene re-derived or a new load — and passes `true` to keep
    /// the pose (re-framed to the new object, `false` to reset to the turntable.
    pub fn install_scene(
        &mut self,
        blob: &[u8],
        label: String,
        preserve_camera: bool,
    ) -> Result<SceneInfo, JsError> {
        let scene =
            scene_transfer::deserialize_scene(blob).map_err(|e| JsError::new(&e.to_string()))?;
        // Drop the old scene's GPU buffers before creating the new ones: GPU
        // residency peaks at one scene instead of two. (The CPU-side old scene
        // is kept until the swap below, so a malformed blob — caught above —
        // costs nothing.) A create failure leaves `None`: frames skip, and the
        // typed error reaches the shell.
        self.renderer = None;
        // A truecolor scene (colours installed into the tree by the deserialize
        // seam) builds the editable *paged* renderer; every other scene builds the
        // palette/static renderer. The paged read is byte-identical to the static
        // truecolor read, so a colour scene renders the same either way.
        let mut renderer = if let Some(pages) = scene.tree.color_pages() {
            GpuRenderer::new_paged(&self.ctx, &scene.structure, &pages)
        } else {
            GpuRenderer::new(&self.ctx, &scene.structure, &scene.table)
        }
        .map_err(|e| JsError::new(&e.to_string()))?;
        if let Some((top, bottom)) = self.sky {
            renderer.set_sky(top, bottom);
        }
        // Re-apply the shell's effect settings — a fresh renderer defaults to
        // shadows ON / Medium, not necessarily what the user chose.
        apply_shadow_quality(&mut renderer, self.shadow_quality);
        renderer.set_gtao(self.gtao_on);
        renderer.set_gtao_params(gtao_params_for(self.gtao_preset));
        self.renderer = Some(renderer);

        // Label conventions: the multi-model caveat (a `.vox` file loads only
        // its first model) and the viewer's TRUECOLOR marker.
        let mut label = label;
        if scene.models_total > 1 {
            label = format!("{label} (model 1 of {})", scene.models_total);
        }
        if scene.tree.has_colors() {
            label = format!("{label} (TRUECOLOR)");
        }
        self.scene_label = label;
        self.resolution = scene.tree.resolution();
        self.nodes = saturating_u32(scene.tree.node_count() as u64);
        self.leaves = saturating_u32(scene.tree.leaf_count() as u64);
        self.voxels = saturating_u32(scene.tree.occupied_voxels());
        self.tree = scene.tree;
        self.structure = scene.structure;
        self.table = scene.table;
        // History cannot survive a scene swap: every image in the ring (and
        // any mid-stroke journal/anchor/self-hit mask) belongs to the old tree.
        self.undo_ring.clear();
        self.stroke.journal.clear();
        self.stroke.mask.clear();
        self.stroke.anchor = None;
        self.stroke.added.clear();
        if preserve_camera {
            self.preserve_camera();
        } else {
            self.reset_camera();
        }
        Ok(self.scene_info())
    }

    /// Snapshots the current scene as a transfer blob, for the IO worker's
    /// encoders (`encode_vox`/`encode_cvox`). A copy — the retained scene
    /// stays live — sized by the scene, cheap next to the off-thread encode.
    pub fn snapshot_scene(&self) -> Vec<u8> {
        // No progress channel on the render thread; the shell's bar covers the
        // worker-side encode that follows.
        scene_transfer::serialize_scene(
            &self.tree,
            &self.structure,
            &self.table,
            1,
            &mut voxel_core::Progress::none(),
        )
    }

    /// Sets the brush configuration (control plane — rare, on a HUD change).
    /// `radius` is capped at [`MAX_BRUSH_RADIUS`], `strength` clamped to `[0, 1]`;
    /// `color_rgba` is sRGB RGBA8 (R low). The next [`brush`](Self::brush) event
    /// reads this.
    pub fn set_brush(
        &mut self,
        tool: BrushTool,
        radius: u32,
        strength: f32,
        falloff: Falloff,
        color_rgba: u32,
        invert: bool,
    ) {
        self.brush_params = BrushParams {
            tool,
            radius: radius.min(MAX_BRUSH_RADIUS),
            strength: strength.clamp(0.0, 1.0),
            falloff,
            color: color_rgba,
            invert,
        };
    }

    /// Sets the render background (control plane — theme boot + theme
    /// changes): a vertical gradient from `top_rgba` to `bottom_rgba` (sRGB
    /// RGBA8, R low), dithered per pixel on the GPU so the subtle ramp never
    /// bands. The shell derives both colours from the live CSS theme tokens,
    /// so the canvas follows the stylesheet. Survives renderer recreation.
    /// Sets the sun-shadow quality: `0` off (the web default), `1` low (the
    /// exact trace at half resolution, bilateral-upsampled — ~4× fewer rays),
    /// `2` high (the exact full-resolution trace). Clamped; applies to the
    /// live renderer and to every renderer rebuilt on a later install.
    pub fn set_shadow_quality(&mut self, quality: u32) {
        self.shadow_quality = quality.min(2);
        if let Some(renderer) = &mut self.renderer {
            apply_shadow_quality(renderer, self.shadow_quality);
        }
    }

    /// Enables/disables the GTAO ambient-occlusion term (on by default). Off
    /// skips the AO + denoise passes entirely — their cost vanishes.
    pub fn set_gtao(&mut self, on: bool) {
        self.gtao_on = on;
        if let Some(renderer) = &mut self.renderer {
            renderer.set_gtao(on);
        }
    }

    /// Sets the GTAO quality preset (`0` Low 1×2, `1` Medium 2×2, `2` High 3×3,
    /// `3` Ultra 9×3 — clamped). Applies now and across scene installs.
    pub fn set_gtao_quality(&mut self, preset: u32) {
        self.gtao_preset = preset.min(3);
        if let Some(renderer) = &mut self.renderer {
            renderer.set_gtao_params(gtao_params_for(self.gtao_preset));
        }
    }

    pub fn set_background(&mut self, top_rgba: u32, bottom_rgba: u32) {
        self.sky = Some((top_rgba, bottom_rgba));
        if let Some(renderer) = &self.renderer {
            renderer.set_sky(top_rgba, bottom_rgba);
        }
    }

    /// Updates the hover pick (data plane — per pointermove with no buttons):
    /// the µs-class CPU raycast the brush uses, driving the GPU cursor ring.
    /// A miss — or negative coordinates, the shell's "pointer left the
    /// canvas" signal — deactivates the ring.
    pub fn hover(&mut self, x: f32, y: f32) {
        let (w, h) = (self.config.width as f32, self.config.height as f32);
        if x < 0.0 || y < 0.0 || w <= 0.0 || h <= 0.0 {
            self.hover = None;
            return;
        }
        let ray = edit::cursor_ray(&self.last_camera, x, y, w, h);
        self.hover = voxel_core::traverse(&self.structure, &ray).map(|hit| {
            #[allow(clippy::cast_precision_loss)] // grid coords ≤ 2048
            [
                hit.voxel.x as f32 + 0.5,
                hit.voxel.y as f32 + 0.5,
                hit.voxel.z as f32 + 0.5,
            ]
        });
    }

    /// Applies one pointer event of the current brush stroke at device-pixel
    /// `(x, y)` with pen `pressure ∈ [0, 1]` (1.0 for a mouse): casts the render
    /// kernel's exact ray, and on a hit stamps the tool's brush — plus
    /// interpolated stamps bridging from the previous event's hit, so a fast drag
    /// is a continuous groove/stroke. In-place events patch only the touched
    /// leaves (occupancy + colour pages) on the GPU; topology-changing events
    /// re-upload the structure once and then the changed colour pages. A miss is a
    /// no-op that breaks the stroke, not an error; the shell calls
    /// [`brush_end`](Self::brush_end) on pointer release.
    pub fn brush(&mut self, x: f32, y: f32, pressure: f32) -> Result<(), JsError> {
        let (w, h) = (self.config.width as f32, self.config.height as f32);
        if w <= 0.0 || h <= 0.0 {
            return Ok(());
        }
        let ray = edit::cursor_ray(&self.last_camera, x, y, w, h);
        let Some(hit) = voxel_core::traverse(&self.structure, &ray) else {
            self.stroke_last = None;
            return Ok(());
        };
        // The promotion threshold (Stage D, locked decision): the FIRST Paint
        // stroke on a palette/fixture scene converts it to per-voxel colour —
        // not scene load, not tool selection — so scenes that are only ever
        // sculpted keep the compact palette path forever. Occupancy is
        // untouched, so the hit stays valid.
        if self.brush_params.tool == BrushTool::Paint && !self.tree.has_colors() {
            self.promote();
        }
        // A hit on this stroke's own fresh material deflects onto the stroke's
        // anchor plane — building volume slides along the surface instead of
        // pillaring toward the camera.
        let picked = edit::resolve_pick(
            &self.stroke,
            &ray,
            hit.voxel,
            self.resolution.voxels_per_axis(),
        );
        let pressure = pressure.clamp(0.0, 1.0);
        let outcome = edit::apply_stroke(
            &mut self.tree,
            &mut self.structure,
            &self.brush_params,
            self.stroke_last,
            edit::StrokeEvent {
                hit: picked,
                pressure,
                // Degenerate-surface fallback for the anchor normal: the
                // brush faces the viewer.
                fallback_normal: -ray.dir,
            },
            &mut self.stroke,
        );
        self.stroke_last = Some(Stamp {
            center: picked,
            pressure,
        });
        if outcome.changed == 0 {
            return Ok(());
        }

        let colored = self.tree.has_colors();
        let Some(renderer) = &mut self.renderer else {
            // No live renderer (failed install): the CPU edit stands; the next
            // successful install re-uploads everything anyway.
            self.needs_full_upload = true;
            self.refresh_counts();
            return Ok(());
        };
        if outcome.topology || self.needs_full_upload {
            // Topology (or a previously failed upload) invalidates the GPU
            // structure buffers wholesale; re-upload once (the colour *pool* is
            // untouched — only the changed pages, synced below). Track failure so
            // a stale renderer is never patched into.
            self.needs_full_upload = if colored {
                renderer.reupload_paged(&self.structure).is_err()
            } else {
                renderer.reupload(&self.structure).is_err()
            };
        } else {
            for &idx in &outcome.touched {
                renderer
                    .update_leaf(&self.structure, idx)
                    .map_err(|e| JsError::new(&e.to_string()))?;
                if !colored {
                    // Palette scenes carry a `leaf_mat` slot; truecolor scenes
                    // sync colour via the pool pages instead (below).
                    renderer
                        .update_leaf_mat(&self.structure, idx)
                        .map_err(|e| JsError::new(&e.to_string()))?;
                }
            }
        }
        // Sync each changed colour page (in-place, or the new/moved pages after a
        // topology re-upload). Bricks that were erased map to no slot and skip.
        if colored && !self.needs_full_upload {
            sync_color_pages(renderer, &self.tree, &outcome.color_bricks)
                .map_err(|e| JsError::new(&e))?;
        }
        self.refresh_counts();
        Ok(())
    }

    /// Ends the active stroke (pointer release): the next `brush` call starts
    /// fresh rather than bridging from the last stroke's endpoint, and the
    /// stroke's journal commits into the undo ring (a stroke that changed
    /// nothing records nothing).
    pub fn brush_end(&mut self) {
        self.stroke_last = None;
        self.stroke.mask.clear();
        self.stroke.anchor = None;
        self.stroke.added.clear();
        if let Some(delta) = self.stroke.journal.commit(&self.tree) {
            self.undo_ring.push(delta);
        }
    }

    /// Undoes the most recent stroke (control plane — `Cmd+Z` / the HUD
    /// button), returning whether anything changed. Restores the stroke's
    /// brick pre-images and re-uploads like one topology stamp, regardless of
    /// how many bricks the stroke touched. A mid-stroke undo commits the
    /// active stroke first, then undoes it.
    pub fn undo(&mut self) -> bool {
        self.time_travel(true)
    }

    /// Re-applies the most recently undone stroke, returning whether anything
    /// changed. The redo stack clears whenever a new stroke lands.
    pub fn redo(&mut self) -> bool {
        self.time_travel(false)
    }

    /// The current scene's identity and counts (control plane; also returned by
    /// the scene-building calls).
    pub fn scene_info(&self) -> SceneInfo {
        SceneInfo {
            label: self.scene_label.clone(),
            nodes: self.nodes,
            leaves: self.leaves,
            voxels: self.voxels,
            res: self.resolution.voxels_per_axis(),
            // Every scene is brush-editable since Stage A3 (the truecolor gate
            // fell); the shell gates only the Paint tool, on `truecolor`.
            editable: true,
            truecolor: self.tree.has_colors(),
        }
    }

    /// Per-frame numbers for the DOM HUD. Data plane: plain `Copy` scalars.
    pub fn stats(&self) -> FrameStats {
        FrameStats {
            frames: self.frames,
            last_dt_ms: self.last_dt_ms,
            nodes: self.nodes,
            leaves: self.leaves,
            voxels: self.voxels,
            res: self.resolution.voxels_per_axis(),
            undo_depth: self.undo_ring.undo_depth(),
            redo_depth: self.undo_ring.redo_depth(),
            truecolor: self.tree.has_colors(),
        }
    }
}

impl Engine {
    /// Re-derives the HUD counts from the live tree (after edits).
    fn refresh_counts(&mut self) {
        self.nodes = saturating_u32(self.tree.node_count() as u64);
        self.leaves = saturating_u32(self.tree.leaf_count() as u64);
        self.voxels = saturating_u32(self.tree.occupied_voxels());
    }

    /// Palette→truecolor promotion (`docs/design/brush-editing/06 §promotion`,
    /// one-way per session): bake every voxel's on-screen colour into the
    /// tree's editable store (table colours for palette voxels, the
    /// position shade for global-0 — pixel-continuous, no visual pop), drop
    /// the dense material store, recreate the renderer on the paged pipeline
    /// (an install-class hitch — the shell's status line covers it), and
    /// clear the undo ring (its images belong to the palette representation).
    fn promote(&mut self) {
        let n = self.resolution.voxels_per_axis();
        let table = &self.table;
        self.tree
            .promote_colors(|c, gid| edit::promotion_color(table, n, c, gid));
        self.structure = SchoolBBuffer::from_sparse(&self.tree);
        // Drop the palette renderer before creating the paged one so GPU
        // residency peaks at one scene; a create failure leaves frames
        // skipping until the next install, like a failed install does.
        self.renderer = None;
        self.renderer = self
            .tree
            .color_pages()
            .and_then(|pages| GpuRenderer::new_paged(&self.ctx, &self.structure, &pages).ok());
        if let Some(renderer) = &mut self.renderer {
            if let Some((top, bottom)) = self.sky {
                renderer.set_sky(top, bottom);
            }
            apply_shadow_quality(renderer, self.shadow_quality);
            renderer.set_gtao(self.gtao_on);
            renderer.set_gtao_params(gtao_params_for(self.gtao_preset));
        }
        self.needs_full_upload = self.renderer.is_none();
        self.undo_ring.clear();
        self.stroke = StrokeState::default();
        self.stroke_last = None;
        if !self.scene_label.contains("(TRUECOLOR)") {
            self.scene_label = format!("{} (TRUECOLOR)", self.scene_label);
        }
    }

    /// The shared undo/redo restore: pop a stroke's images from the ring,
    /// restore them into the tree, rebuild the structure once, and re-sync the
    /// GPU exactly like a topology stamp (full structure re-upload + the
    /// restored bricks' colour pages). GPU failures latch `needs_full_upload`,
    /// healing on the next edit like a failed brush upload does.
    fn time_travel(&mut self, back: bool) -> bool {
        // A dangling mid-stroke journal commits first, so an undo during a
        // held drag undoes that partial stroke instead of fighting it.
        self.brush_end();
        let restore = if back {
            self.undo_ring.undo()
        } else {
            self.undo_ring.redo()
        };
        let Some(restore) = restore else {
            return false;
        };
        let codes: Vec<u64> = restore.iter().map(|&(code, _)| code).collect();
        self.tree.replace_bricks(restore);
        self.structure = SchoolBBuffer::from_sparse(&self.tree);
        let colored = self.tree.has_colors();
        if let Some(renderer) = &mut self.renderer {
            self.needs_full_upload = if colored {
                renderer.reupload_paged(&self.structure).is_err()
            } else {
                renderer.reupload(&self.structure).is_err()
            };
            if colored && !self.needs_full_upload {
                self.needs_full_upload = sync_color_pages(renderer, &self.tree, &codes).is_err();
            }
        } else {
            self.needs_full_upload = true;
        }
        self.refresh_counts();
        true
    }

    /// Advances whichever camera is live and packs the frame's uniform.
    fn update_camera(&mut self, dt: f32) -> GpuCamera {
        let (w, h) = (self.config.width, self.config.height);
        let n = self.resolution.voxels_per_axis() as f32;
        let k = self.resolution.internal_levels();
        match self.mode {
            CameraMode::Orbit => {
                self.orbit.integrate(dt);
                self.orbit.to_gpu(self.orbit_frame, w, h, n, k)
            }
            CameraMode::Fly => {
                self.fly.apply(dt, &self.input);
                self.fly.to_gpu(w, h, n, k)
            }
        }
    }

    /// Resets to the ambient-spinning orbit framing the current scene — the
    /// state a new scene load starts from. Recomputes the orbit frame from the
    /// (already-swapped) tree so the pivot follows the new object.
    fn reset_camera(&mut self) {
        self.frames = 0;
        self.mode = CameraMode::Orbit;
        let n = self.resolution.voxels_per_axis() as f32;
        let bbox = self.tree.occupied_bbox();
        self.orbit_frame = OrbitFrame::for_bbox(bbox, n);
        let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        self.orbit = OrbitControl::default().with_radius(fit_orbit_radius(bbox, aspect));
        let (eye, fwd) = self.orbit.eye_forward(self.orbit_frame);
        self.fly = FlyCamera::from_eye_forward(eye, fwd, n);
        self.input = Input::default();
        self.last_camera = self.orbit.to_gpu(
            self.orbit_frame,
            self.config.width,
            self.config.height,
            n,
            self.resolution.internal_levels(),
        );
        self.stroke_last = None;
        self.needs_full_upload = false;
    }

    /// Keeps the current view across a scene re-derivation. The orbit is
    /// scale-invariant angles + the per-scene frame, so it re-frames to the new
    /// object with no remap; the fly pose is remapped by the old→new frame
    /// similarity (see [`FlyCamera::reframed`]) so a resolution change moves
    /// the camera with the scene rather than leaving it stranded. `mode`,
    /// `input`, and the frame counter all persist; only the scene-derived
    /// buffers refresh.
    fn preserve_camera(&mut self) {
        let n = self.resolution.voxels_per_axis() as f32;
        let (w, h) = (self.config.width, self.config.height);
        let k = self.resolution.internal_levels();
        let old_frame = self.orbit_frame;
        let new_frame = OrbitFrame::for_bbox(self.tree.occupied_bbox(), n);
        // The interactive orbit is scale-invariant (radius is an extent
        // fraction), so it needs no remap; only the fly pose does.
        self.fly = self.fly.reframed(old_frame, new_frame);
        self.orbit_frame = new_frame;
        self.last_camera = match self.mode {
            CameraMode::Orbit => self.orbit.to_gpu(self.orbit_frame, w, h, n, k),
            CameraMode::Fly => self.fly.to_gpu(w, h, n, k),
        };
        // Topology changed under the camera; the next edit re-uploads, and any
        // in-flight stroke can no longer bridge into the new leaf layout.
        self.stroke_last = None;
        self.needs_full_upload = false;
    }
}

/// Re-uploads the colour pages of the bricks in `codes` (post-edit or
/// post-restore): pack each brick's current page and write it plus its
/// page-table word. Bricks that no longer exist (erased) map to no slot and
/// skip. Shared by the brush sync and the undo/redo restore.
fn sync_color_pages(
    renderer: &mut GpuRenderer,
    tree: &SparseTree,
    codes: &[u64],
) -> Result<(), String> {
    for &code in codes {
        let Some(slot) = edit::slot_of_brick(tree, code) else {
            continue;
        };
        let pages = tree.color_pages().expect("truecolor scene");
        let s = slot as usize;
        let page = pages.page_of(s);
        let words = voxel_core::color_pool::pack_page(pages.colors_of(s), pages.class_of(s));
        renderer
            .update_color_page(u64::from(page), &words)
            .map_err(|e| e.to_string())?;
        renderer
            .update_page_word(slot, page)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Clamps a wide count into `u32` for the stats structs (a count above
/// `u32::MAX` is not representable and reads as saturated).
fn saturating_u32(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}
