//! The IO worker's `#[wasm_bindgen]` surface: every scene *build* and *codec*
//! job, off the main thread (`docs/design/web-frontend-api.md` §5, stage 7).
//!
//! `IoKernel` owns its **own** WebGPU device (WebGPU objects cannot cross
//! workers) — legitimate here because the pipeline already round-trips through
//! the CPU between voxelization and rendering, so nothing GPU-resident is
//! lost. Every job returns (or consumes) the [`crate::scene_transfer`] blob:
//! one buffer, one `postMessage` transferable, zero-copy across contexts.
//!
//! Jobs are synchronous or GPU-async; the worker script processes messages
//! serially, so no two jobs ever contend for the kernel's bindgen borrow.

use voxel_core::{MaterialTable, Progress, Resolution};
use voxel_gpu::GpuContext;
use wasm_bindgen::prelude::*;

use crate::mesh::{MeshBuildOptions, MeshKind};
use crate::phases::Phase;
use crate::{mesh, scene, scene_transfer, vox};

/// The mesh container format of bytes handed to [`IoKernel::voxelize_mesh`].
/// GLB is the primary web format (self-contained); OBJ is geometry-only on the
/// web (`docs/design/web-frontend-api.md` §8).
#[wasm_bindgen]
#[derive(Clone, Copy)]
pub enum MeshFormat {
    /// Binary glTF (`.glb`) — self-contained, textures included.
    Glb,
    /// Wavefront OBJ (`.obj`) — geometry only on the web.
    Obj,
    /// STL (`.stl`).
    Stl,
}

/// Control-plane options for [`IoKernel::voxelize_mesh`], deserialized from a
/// plain JS object. Defaults mirror the native viewer's `--mesh` arguments.
#[derive(serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct VoxelizeMeshOptions {
    /// Grid resolution per axis (a legal `8·4^k` size).
    res: u32,
    /// Corrective rotation about X, degrees (e.g. `-90` for Z-up OBJ/STL).
    rot_x: f32,
    /// Corrective rotation about Y, degrees.
    rot_y: f32,
    /// Corrective rotation about Z, degrees.
    rot_z: f32,
    /// Voxels of margin around the mesh's bounding box when fitting the grid.
    padding: f32,
    /// Bake per-voxel truecolor when the mesh is textured.
    truecolor: bool,
    /// Bake colours on the GPU (CPU-oracle fallback; `false` = A/B path).
    gpu_bake: bool,
}

impl Default for VoxelizeMeshOptions {
    fn default() -> Self {
        Self {
            res: 128,
            rot_x: 0.0,
            rot_y: 0.0,
            rot_z: 0.0,
            padding: 2.0,
            truecolor: true,
            gpu_bake: true,
        }
    }
}

/// Control-plane options for the `.vox`/`.cvox` decoders.
#[derive(Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct DecodeVoxOptions {
    /// Grid resolution per axis; auto-sized to the smallest legal grid that
    /// holds the model when absent.
    res: Option<u32>,
}

/// The scene-building and codec kernel the IO worker instantiates.
#[wasm_bindgen]
pub struct IoKernel {
    ctx: GpuContext,
}

#[wasm_bindgen]
impl IoKernel {
    /// Async init: requests this worker's own adapter/device (no surface —
    /// this kernel never presents).
    pub async fn create() -> Result<IoKernel, JsError> {
        console_error_panic_hook::set_once();
        let ctx = GpuContext::try_new_async()
            .await
            .map_err(|e| JsError::new(&format!("no WebGPU adapter in worker: {e}")))?;
        Ok(IoKernel { ctx })
    }

    /// Voxelizes mesh bytes into a scene blob (GLB primary; OBJ geometry-only;
    /// STL binary/ASCII). The sparse GPU voxelize + optional MASK-cutout +
    /// truecolor bake, async end-to-end on this worker's device. `opts`:
    /// `{ res?, rotX?, rotY?, rotZ?, padding?, truecolor? }`.
    ///
    /// `on_progress` is invoked as `(phase_key, done, total)` on the meters'
    /// schedule (`total = 0` marks an indeterminate phase) — worker→main
    /// messages deliver even while this thread crunches a synchronous phase,
    /// so the shell's bar stays live through the bake.
    pub async fn voxelize_mesh(
        &self,
        bytes: Vec<u8>,
        format: MeshFormat,
        opts: JsValue,
        on_progress: js_sys::Function,
    ) -> Result<Vec<u8>, JsError> {
        let opts: VoxelizeMeshOptions = parse_opts(opts)?;
        let resolution = Resolution::new(opts.res).map_err(|e| JsError::new(&e.to_string()))?;
        let kind = match format {
            MeshFormat::Glb => MeshKind::Gltf,
            MeshFormat::Obj => MeshKind::Obj,
            MeshFormat::Stl => MeshKind::Stl,
        };
        let build_opts = MeshBuildOptions {
            rotation: voxelizer::rotation_degrees(opts.rot_x, opts.rot_y, opts.rot_z),
            padding: opts.padding,
            truecolor: opts.truecolor,
            gpu_bake: opts.gpu_bake,
        };
        let mut report = progress_reporter(on_progress);
        let built = mesh::build_from_mesh_bytes(
            &self.ctx.device,
            &self.ctx.queue,
            resolution,
            bytes, // ownership handed over: the builder frees the copy post-parse
            kind,
            &build_opts,
            &mut report,
        )
        .await
        .map_err(|e| JsError::new(&e.to_string()))?;
        scene_transfer::check_scene_budget(&built.tree, &built.structure)
            .map_err(|e| JsError::new(&e.to_string()))?;
        let mut pack = |done, total| report(Phase::Pack, done, total);
        Ok(scene_transfer::serialize_scene(
            &built.tree,
            &built.structure,
            &built.table,
            1,
            &mut Progress::new(&mut pack),
        ))
    }

    /// Builds a named fixture into a scene blob. CPU fixtures build directly;
    /// the noise fixtures (`perlin`, `caves`) evaluate their occupancy on this
    /// worker's device via the async brick-compaction generator. `on_progress`
    /// is invoked as `(phase_key, done, total)` — the `generate`/`assemble`
    /// phase pair (both indeterminate).
    pub async fn build_fixture(
        &self,
        fixture: String,
        res: u32,
        on_progress: js_sys::Function,
    ) -> Result<Vec<u8>, JsError> {
        let resolution = Resolution::new(res).map_err(|e| JsError::new(&e.to_string()))?;
        let mut report = progress_reporter(on_progress);
        let (tree, structure) =
            scene::build_fixture_gpu(&self.ctx, &fixture, resolution, &mut report)
                .await
                .map_err(|e| JsError::new(&e.to_string()))?;
        scene_transfer::check_scene_budget(&tree, &structure)
            .map_err(|e| JsError::new(&e.to_string()))?;
        let mut pack = |done, total| report(Phase::Pack, done, total);
        Ok(scene_transfer::serialize_scene(
            &tree,
            &structure,
            &MaterialTable::missing_only(),
            1,
            &mut Progress::new(&mut pack),
        ))
    }

    /// Decodes `MagicaVoxel` `.vox` bytes into a scene blob (first model of a
    /// multi-model file; the blob carries the model count for the label
    /// caveat). `opts`: `{ res? }`; `on_progress` reports `parse`/`assemble`.
    #[allow(clippy::unused_self)]
    pub fn decode_vox(
        &self,
        bytes: &[u8],
        opts: JsValue,
        on_progress: js_sys::Function,
    ) -> Result<Vec<u8>, JsError> {
        let opts: DecodeVoxOptions = parse_opts(opts)?;
        let mut report = progress_reporter(on_progress);
        let scene = vox::import_vox(bytes, opts.res, &mut report)
            .map_err(|e| JsError::new(&e.to_string()))?;
        scene_transfer::check_scene_budget(&scene.tree, &scene.structure)
            .map_err(|e| JsError::new(&e.to_string()))?;
        let models = u32::try_from(scene.models_total).unwrap_or(u32::MAX);
        let mut pack = |done, total| report(Phase::Pack, done, total);
        Ok(scene_transfer::serialize_scene(
            &scene.tree,
            &scene.structure,
            &scene.table,
            models,
            &mut Progress::new(&mut pack),
        ))
    }

    /// Decodes compressed `.cvox` bytes into a scene blob (every model merged
    /// — no caveat to carry). `opts`: `{ res? }`; `on_progress` reports
    /// `parse`/`assemble`.
    #[allow(clippy::unused_self)]
    pub fn decode_cvox(
        &self,
        bytes: &[u8],
        opts: JsValue,
        on_progress: js_sys::Function,
    ) -> Result<Vec<u8>, JsError> {
        let opts: DecodeVoxOptions = parse_opts(opts)?;
        let mut report = progress_reporter(on_progress);
        let scene = vox::import_cvox(bytes, opts.res, &mut report)
            .map_err(|e| JsError::new(&e.to_string()))?;
        scene_transfer::check_scene_budget(&scene.tree, &scene.structure)
            .map_err(|e| JsError::new(&e.to_string()))?;
        let mut pack = |done, total| report(Phase::Pack, done, total);
        Ok(scene_transfer::serialize_scene(
            &scene.tree,
            &scene.structure,
            &scene.table,
            1,
            &mut Progress::new(&mut pack),
        ))
    }

    /// Encodes a scene blob (from [`Engine::snapshot_scene`]) as `.vox` bytes.
    /// `on_progress` reports the `gather`/`write` phase pair.
    ///
    /// [`Engine::snapshot_scene`]: crate::Engine::snapshot_scene
    #[allow(clippy::unused_self)]
    pub fn encode_vox(
        &self,
        scene_blob: &[u8],
        on_progress: js_sys::Function,
    ) -> Result<Vec<u8>, JsError> {
        let s = scene_transfer::deserialize_scene(scene_blob)
            .map_err(|e| JsError::new(&e.to_string()))?;
        let mut report = progress_reporter(on_progress);
        vox::export_vox(&s.tree, &s.structure, &s.table, &mut report)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// Encodes a scene blob as compressed `.cvox` bytes. `on_progress` reports
    /// the `gather`/`write` phase pair.
    #[allow(clippy::unused_self)]
    pub fn encode_cvox(
        &self,
        scene_blob: &[u8],
        on_progress: js_sys::Function,
    ) -> Result<Vec<u8>, JsError> {
        let s = scene_transfer::deserialize_scene(scene_blob)
            .map_err(|e| JsError::new(&e.to_string()))?;
        let mut report = progress_reporter(on_progress);
        vox::export_cvox(&s.tree, &s.structure, &s.table, &mut report)
            .map_err(|e| JsError::new(&e.to_string()))
    }
}

/// Curries a JS `(phaseKey, done, total)` callback into the kernel's
/// `(Phase, u64, u64)` progress-sink shape (the one channel all jobs share).
/// Takes the bindgen-owned `Function` by value and moves it into the returned
/// sink. A failing callback never fails the job — the error is dropped.
#[allow(clippy::cast_precision_loss)] // counts ≤ 2^53 by construction
fn progress_reporter(on_progress: js_sys::Function) -> impl FnMut(Phase, u64, u64) {
    move |phase: Phase, done: u64, total: u64| {
        let _ = on_progress.call3(
            &JsValue::NULL,
            &JsValue::from_str(phase.key()),
            &JsValue::from_f64(done as f64),
            &JsValue::from_f64(total as f64),
        );
    }
}

/// Deserializes an optional control-plane options object.
fn parse_opts<T: Default + serde::de::DeserializeOwned>(opts: JsValue) -> Result<T, JsError> {
    if opts.is_undefined() || opts.is_null() {
        Ok(T::default())
    } else {
        serde_wasm_bindgen::from_value(opts).map_err(|e| JsError::new(&e.to_string()))
    }
}
