//! The GPU-resident render path: one compute dispatch builds a camera ray per
//! pixel, traverses, shades the hit, and writes color straight to a storage
//! texture — no readback, no CPU ray-gen, no CPU shading. The viewer blits the
//! resulting texture to its surface.

// Unsafe Quarantine: the only `unsafe` is the `bytemuck` derive on the
// `#[repr(C)]` all-scalar camera uniform.
#![allow(unsafe_code)]

use bytemuck::{Pod, Zeroable};

use voxel_core::{MaterialTable, NodeLayout, SchoolBBuffer};

use crate::buffers;
use crate::context::GpuContext;
use crate::error::GpuError;
use voxel_core::GpuCamera;
use wgpu::util::DeviceExt;

/// The output storage-texture format the render kernel writes.
pub const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// GTAO quality/tuning uniform: the slice & step counts (the quality preset —
/// Low 1×2, Medium 2×2, High 3×3, Ultra 9×3) and the effect radius in voxels.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GtaoParams {
    /// Slices per pixel.
    pub slice_count: f32,
    /// Horizon-march steps per slice.
    pub steps_per_slice: f32,
    /// Effect radius in voxels (world units).
    pub effect_radius: f32,
    /// Per-frame index that rotates the sample noise (set by the renderer each
    /// frame; ignored when passed to [`GpuRenderer::set_gtao_params`]).
    pub frame_index: u32,
}

impl Default for GtaoParams {
    fn default() -> Self {
        // Medium preset (2×2), radius 22 voxels — the intended production target
        // once temporal accumulation cleans the lower sample count.
        Self {
            slice_count: 2.0,
            steps_per_slice: 2.0,
            effect_radius: 22.0,
            frame_index: 0,
        }
    }
}

/// Per-frame TAA uniform (mirrors `gtao_taa.wgsl`'s `TaaParams`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct TaaParams {
    width: u32,
    height: u32,
    frame_index: u32,
    history_valid: u32,
}

/// Per-denoise-pass uniform: blur strength + viewport size.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct DenoiseParams {
    blur_beta: f32,
    final_apply: u32,
    width: u32,
    height: u32,
}

/// Number of render passes individually GPU-timed, each bracketed with its own
/// begin/end timestamp (the Metal-portable pattern). In pass order.
pub const NUM_TIMED_STAGES: usize = 7;
/// Short labels for the timed stages ([`NUM_TIMED_STAGES`]). Index matches
/// [`GpuRenderer::last_stage_times_ns`], not execution order. `BLEND` dispatches
/// nothing (≈0 ns) on scenes without transparency, so every slot stays valid.
pub const RENDER_STAGE_LABELS: [&str; NUM_TIMED_STAGES] =
    ["GBUF", "GTAO", "DNS1", "DNS2", "COMP", "TAA", "BLEND"];

/// Reusable compute-pass timestamp resources (present iff the device supports
/// `TIMESTAMP_QUERY`): a `2·NUM_TIMED_STAGES`-slot query set (begin/end per pass)
/// and the resolve/readback buffers. Measured on the GPU timeline (a small
/// timestamp readback, no per-pixel copy).
struct RenderTiming {
    query_set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    /// Nanoseconds per timestamp tick.
    period: f32,
}

/// Eye-space linear depth (perpendicular, along the view forward) — the
/// G-buffer depth format. Storage-writable in the G-buffer pass, sampled in GTAO
/// + composite.
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// World-space face normal, encoded `*0.5+0.5` — the G-buffer normal format.
const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Per-voxel truecolor albedo (sRGB RGBA8) — written by the G-buffer, read by
/// composite (gated by `dims.w` bit7). The deferred truecolor albedo source.
const ALBEDO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// GTAO visibility term in `[0,1]` — the AO texture format. `R32Float` because
/// single-channel 8-bit formats aren't WebGPU storage-writable; the precision is
/// harmless overkill. The packed depth-edge term shares the same format.
const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// Denoise blur strength (the `DenoiseBlurBeta` default); the sharp first pass
/// uses `/5`, the soft second pass the full value.
const DENOISE_BLUR_BETA: f32 = 1.2;

/// Fixed sun elevation component (the `y` of the un-normalized direction); lower
/// = longer, more dramatic sweeping shadows.
const SUN_ELEVATION: f32 = 0.6;

/// Composited-colour + TAA-history format — `Rgba16Float` for precision across
/// the temporal accumulation (8-bit would band over many frames).
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Viewport-sized intermediate textures for the multi-pass shading pipeline,
/// recreated when the viewport size changes. Each has both `STORAGE_BINDING`
/// (written by an earlier pass) and `TEXTURE_BINDING` (read via `textureLoad` by
/// a later pass).
struct GBuf {
    width: u32,
    height: u32,
    #[allow(dead_code)] // kept alive; bound via the views
    depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    #[allow(dead_code)]
    normal: wgpu::Texture,
    normal_view: wgpu::TextureView,
    #[allow(dead_code)]
    albedo: wgpu::Texture,
    albedo_view: wgpu::TextureView, // per-voxel truecolor albedo (gbuffer → composite)
    #[allow(dead_code)]
    ao: wgpu::Texture,
    ao_view: wgpu::TextureView, // GTAO main-pass output (raw AO)
    #[allow(dead_code)]
    edges: wgpu::Texture,
    edges_view: wgpu::TextureView, // packed depth edges (denoise weights)
    #[allow(dead_code)]
    ao_pong: wgpu::Texture,
    ao_pong_view: wgpu::TextureView, // denoise pass-1 output
    #[allow(dead_code)]
    ao_denoised: wgpu::Texture,
    ao_denoised_view: wgpu::TextureView, // denoise pass-2 output → composite
    #[allow(dead_code)]
    color: wgpu::Texture,
    color_view: wgpu::TextureView, // composite output (pre-TAA, HDR-ish)
    #[allow(dead_code)]
    color_blended: wgpu::Texture,
    color_blended_view: wgpu::TextureView, // transparents composited over color (pre-TAA)
    #[allow(dead_code)]
    history: [wgpu::Texture; 2],
    history_view: [wgpu::TextureView; 2], // TAA colour-history ping-pong
    #[allow(dead_code)]
    prev_depth: wgpu::Texture,
    prev_depth_view: wgpu::TextureView, // last frame's depth (for reprojection)
}

/// Which per-voxel albedo source the deferred g-buffer writes for a scene — main's
/// three mutually-exclusive shading modes, chosen once at [`GpuRenderer::new`]:
/// `Truecolor` when the structure carries baked colour, `Palette` when it carries
/// material indices + a non-trivial table, else `None` (occupancy → position-hash).
#[derive(Clone, Copy, PartialEq, Eq)]
enum AlbedoMode {
    Truecolor,
    Palette,
    None,
}

/// A compiled multi-pass shading pipeline (G-buffer → composite) with one
/// uploaded structure. GTAO will add an AO pass between the two.
// Independent debug toggles (history-valid, denoise, shadows, sun-anim) — flags,
// not a state machine, so a struct of bools is right.
#[allow(clippy::struct_excessive_bools)]
pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    gbuffer_pipeline: wgpu::ComputePipeline,
    gbuffer_layout: wgpu::BindGroupLayout,
    /// The skip-transparent g-buffer variant (Stage C): `traverse_ray_opaque` over the
    /// same truecolor layout, selected at record time iff `has_transparency`. Built
    /// unconditionally so it always naga-validates.
    gbuffer_skip_pipeline: wgpu::ComputePipeline,
    /// The palette-mode g-buffer variant (binds `leaf_mat`@8 + `material_table`@9
    /// instead of the truecolor colour buffers); selected at record time by
    /// `albedo_mode`. Built unconditionally so it always naga-validates.
    palette_gbuffer_pipeline: wgpu::ComputePipeline,
    palette_gbuffer_layout: wgpu::BindGroupLayout,
    gtao_pipeline: wgpu::ComputePipeline,
    gtao_layout: wgpu::BindGroupLayout,
    denoise_pipeline: wgpu::ComputePipeline,
    denoise_layout: wgpu::BindGroupLayout,
    denoise_params1_buf: wgpu::Buffer, // sharp pass (beta/5)
    denoise_params2_buf: wgpu::Buffer, // soft pass (beta)
    composite_pipeline: wgpu::ComputePipeline,
    composite_layout: wgpu::BindGroupLayout,
    taa_pipeline: wgpu::ComputePipeline,
    taa_layout: wgpu::BindGroupLayout,
    /// Forward-blend transparency pass (Stage C): composites transparent voxels over
    /// the lit composite result into `color_blended`, dispatched only when
    /// `has_transparency`. Built unconditionally so it always naga-validates.
    blend_pipeline: wgpu::ComputePipeline,
    blend_layout: wgpu::BindGroupLayout,
    /// Lazily (re)created viewport-sized G-buffer textures.
    gbuf: Option<GBuf>,
    node_buf: wgpu::Buffer,
    leaf_buf: wgpu::Buffer,
    bounds_buf: wgpu::Buffer,
    /// Truecolor per-voxel colour: base offsets + up to `N_MAX_CHUNKS` colour chunks
    /// (or 1-`u32` dummies for non-truecolor scenes), bound @8..11 of the truecolor
    /// g-buffer pipeline.
    leaf_color_base_buf: wgpu::Buffer,
    leaf_color_chunks: Vec<wgpu::Buffer>,
    color_dummy_buf: wgpu::Buffer,
    /// The colour-chunk capacity the pipelines were compiled with (`PER_CHUNK`):
    /// [`buffers::COLOR_PER_CHUNK`] for a static build, the pool's
    /// `chunk_entries` for a paged one — CPU page placement and GPU chunk-select
    /// stay aligned by construction.
    per_chunk: u32,
    /// Whether the colour buffers are the editable paged pool (brush editing) —
    /// gates the [`update_color_page`](Self::update_color_page) surface.
    color_editable: bool,
    /// Palette materials: the packed per-leaf `leaf_mat`@8 + the global
    /// `material_table`@9 (or 1-`u32` dummies for non-palette scenes, never bound),
    /// bound by the palette g-buffer pipeline.
    leaf_mat_buf: wgpu::Buffer,
    material_table_buf: wgpu::Buffer,
    /// Which albedo source this scene uses — selects the g-buffer pipeline and gates
    /// `dims.w` (bit7 truecolor / bit2 palette; neither for `None`).
    albedo_mode: AlbedoMode,
    /// Whether this scene has semi-transparent voxels — selects the forward-blend pass
    /// + the TAA colour input. `has_transparency ⟹ has_leaf_color ⟹ Truecolor`.
    has_transparency: bool,
    camera_buf: wgpu::Buffer,
    /// The hover-cursor uniform (@9 of the TAA pass). Zero — WebGPU
    /// zero-initializes buffers — means inactive, so a renderer that never
    /// calls [`set_cursor`](Self::set_cursor) renders byte-identically to a
    /// cursor-free build.
    cursor_buf: wgpu::Buffer,
    /// The themed-sky uniform (@10 of the composite pass), initialized to the
    /// product's original gradient so a renderer that never calls
    /// [`set_sky`](Self::set_sky) looks as it always did.
    sky_buf: wgpu::Buffer,
    prev_camera_buf: wgpu::Buffer,
    gtao_params_buf: wgpu::Buffer,
    taa_params_buf: wgpu::Buffer,
    /// User-set GTAO quality (the per-frame `frame_index` is filled in by the
    /// renderer); the live noise/TAA state.
    gtao_quality: GtaoParams,
    prev_camera: GpuCamera,
    frame_index: u32,
    /// False until a same-size prior frame exists (first frame / after resize).
    history_valid: bool,
    /// Toggle (default on): the GTAO term itself. When off, the GTAO + denoise
    /// dispatches are skipped entirely (their cost vanishes) and composite reads
    /// a constant full-visibility AO via the dims.w bit3 gate.
    gtao_enabled: bool,
    /// Debug toggle (default on): when off, composite reads the raw GTAO AO and the
    /// TAA passes through (no spatial denoise, no temporal accumulation).
    denoise: bool,
    /// Debug toggle (default on): when off, the G-buffer skips the sun shadow ray
    /// (everything reads as lit) — removes the sharp ray-traced umbras.
    shadows: bool,
    /// Toggle (default off): trace the shadow with the coarse brick-level
    /// `traverse_occluded` instead of the exact `traverse_ray` — cheaper,
    /// slightly fatter/blockier umbras.
    coarse_shadows: bool,
    /// Sun direction uniform (`vec4`: xyz dir + pad), shared by the G-buffer shadow
    /// ray and the composite direct-light term so they always agree.
    sun_buf: wgpu::Buffer,
    /// Sun azimuth (radians), static — set via [`set_sun_phase`](Self::set_sun_phase).
    sun_phase: f32,
    /// Max storage-buffer binding size, kept so [`reupload`](Self::reupload) can
    /// rebuild the structure buffers after a topology edit without the context.
    max_binding: u64,
    timing: Option<RenderTiming>,
}

/// A write-only storage-texture bind-group-layout entry of the given format.
fn storage_tex_entry(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

/// A 1-`u32` STORAGE buffer for colour/material bind slots a scene's albedo mode
/// doesn't use (wgpu rejects zero-sized storage bindings; never bound by the
/// selected pipeline).
fn dummy_storage(device: &wgpu::Device, label: &str) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: 4,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    })
}

/// A sampled (`textureLoad`-able) float texture bind-group-layout entry.
fn sampled_tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// The flat colour-pool image the GPU chunks tile: each leaf's rank-order colours
/// placed at its page offset (padding entries left zero — never read, since a
/// voxel's rank is always `< occupancy ≤ class`). This is the CPU mirror of what
/// the GPU pool holds, built once at [`GpuRenderer::new_paged`].
fn build_pool_image(pages: &voxel_core::ColorPages) -> Vec<u32> {
    let mut image = vec![0u32; usize::try_from(pages.total_entries()).unwrap_or(0)];
    for i in 0..pages.len() {
        let base = pages.page_of(i) as usize;
        let colors = pages.colors_of(i);
        image[base..base + colors.len()].copy_from_slice(colors);
    }
    image
}

impl GpuRenderer {
    /// Compiles the multi-pass shading kernels (G-buffer + composite) and uploads
    /// `structure`.
    #[allow(clippy::too_many_lines)] // straight-line setup of two pipelines
    pub fn new(
        ctx: &GpuContext,
        structure: &SchoolBBuffer,
        table: &MaterialTable,
    ) -> Result<Self, GpuError> {
        Self::new_with_per_chunk(ctx, structure, table, buffers::COLOR_PER_CHUNK)
    }

    /// [`new`](Self::new) with an explicit colour-chunk size — for tests that force
    /// a tiny `per_chunk` to drive the `N > 1` cross-chunk path on a small scene
    /// (no 285 MiB of VRAM needed). Production calls [`new`](Self::new).
    ///
    /// # Errors
    /// [`GpuError`] as [`new`](Self::new).
    #[doc(hidden)]
    pub fn new_with_per_chunk(
        ctx: &GpuContext,
        structure: &SchoolBBuffer,
        table: &MaterialTable,
        per_chunk: u32,
    ) -> Result<Self, GpuError> {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();

        let max_binding = ctx.max_storage_binding();
        let (node_buf, leaf_buf, bounds_buf) =
            buffers::upload_structure(&device, structure, max_binding)?;
        // Albedo source for this scene (main's three mutually-exclusive modes).
        let albedo_mode = if structure.has_leaf_color() {
            AlbedoMode::Truecolor
        } else if !structure.leaf_mat_words().is_empty() || table.words().len() > 1 {
            AlbedoMode::Palette
        } else {
            AlbedoMode::None
        };
        let has_transparency = structure.has_transparency();
        // Truecolor colour buffers (bound by the truecolor/None g-buffer). Probe first
        // so a failure leaves no partial GPU state; dummies otherwise.
        let (leaf_color_base_buf, leaf_color_chunks, color_dummy_buf) =
            if albedo_mode == AlbedoMode::Truecolor {
                buffers::probe_truecolor(
                    structure.resolution().voxels_per_axis(),
                    structure.leaf_color_words().len(),
                    (structure.leaf_color_base_words().len() * 4) as u64,
                    per_chunk,
                    ctx.max_storage_buffers(),
                    max_binding,
                    ctx.max_buffer_size(),
                )?;
                let (chunks, base, dummy) = buffers::upload_color_chunks(
                    &device,
                    structure.leaf_color_words(),
                    structure.leaf_color_base_words(),
                    per_chunk,
                );
                (base, chunks, dummy)
            } else {
                (
                    dummy_storage(&device, "color base dummy"),
                    Vec::new(),
                    dummy_storage(&device, "color chunk dummy"),
                )
            };
        // Palette material buffers (bound by the palette g-buffer); real for Palette,
        // else dummies that are never bound.
        let (leaf_mat_buf, material_table_buf) = if albedo_mode == AlbedoMode::Palette {
            buffers::upload_materials(&device, structure, table, max_binding)?
        } else {
            (
                dummy_storage(&device, "leaf_mat dummy"),
                dummy_storage(&device, "material_table dummy"),
            )
        };
        Ok(Self::assemble_deferred(
            ctx,
            device,
            queue,
            node_buf,
            leaf_buf,
            bounds_buf,
            albedo_mode,
            has_transparency,
            leaf_color_base_buf,
            leaf_color_chunks,
            color_dummy_buf,
            per_chunk,
            false,
            leaf_mat_buf,
            material_table_buf,
            max_binding,
        ))
    }

    /// Builds an **editable paged** truecolor renderer (brush-editing Stage A2)
    /// from a tree's colour pool ([`SparseTree::color_pages`]). `structure` is the
    /// [`SchoolBBuffer::from_sparse`] of the same tree (it supplies the derived
    /// `leaf_color_page` table and the transparency bits). The hit-read is
    /// byte-identical to [`new`](Self::new)'s static path — only the page offsets
    /// (editable pool pages vs prefix sums) and the buffer usages (`COPY_DST`,
    /// growable) differ — so a paged scene shades identically to its build-once
    /// bake while accepting in-place [`update_color_page`](Self::update_color_page)
    /// / [`reupload_paged`](Self::reupload_paged) edits.
    ///
    /// [`SparseTree::color_pages`]: voxel_core::SparseTree::color_pages
    ///
    /// # Errors
    /// [`GpuError`] as [`new`](Self::new): a structure buffer over the binding cap,
    /// or the colour pool exceeding the compiled chunk count / device limits.
    pub fn new_paged(
        ctx: &GpuContext,
        structure: &SchoolBBuffer,
        pages: &voxel_core::ColorPages,
    ) -> Result<Self, GpuError> {
        let n_res = structure.resolution().voxels_per_axis();
        let per_chunk =
            u32::try_from(pages.chunk_entries()).map_err(|_| GpuError::Unsupported {
                n: n_res,
                reason: "colour pool chunk size exceeds u32",
            })?;
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        let max_binding = ctx.max_storage_binding();
        let (node_buf, leaf_buf, bounds_buf) =
            buffers::upload_structure(&device, structure, max_binding)?;
        let pool_image = build_pool_image(pages);
        let page_words = structure.leaf_color_page_words();
        buffers::probe_truecolor(
            n_res,
            pool_image.len(),
            (page_words.len() * 4) as u64,
            per_chunk,
            ctx.max_storage_buffers(),
            max_binding,
            ctx.max_buffer_size(),
        )?;
        let (leaf_color_chunks, page_buf, color_dummy_buf) =
            buffers::upload_color_pool(&device, &pool_image, page_words, per_chunk);
        // The paged pool carries colour by definition; transparency comes from the
        // tree's colour store (from_sparse derived the bounds bits from it).
        let has_transparency = pages.has_transparency();
        let leaf_mat_buf = dummy_storage(&device, "leaf_mat dummy");
        let material_table_buf = dummy_storage(&device, "material_table dummy");
        Ok(Self::assemble_deferred(
            ctx,
            device,
            queue,
            node_buf,
            leaf_buf,
            bounds_buf,
            AlbedoMode::Truecolor,
            has_transparency,
            page_buf,
            leaf_color_chunks,
            color_dummy_buf,
            per_chunk,
            true,
            leaf_mat_buf,
            material_table_buf,
            max_binding,
        ))
    }

    /// The shared construction tail: compiles every pass pipeline (the colour
    /// assemblies with `per_chunk` injected), creates the uniforms, and wires the
    /// optional GPU-timestamp resources. Both constructors funnel here.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // straight-line setup
    fn assemble_deferred(
        ctx: &GpuContext,
        device: wgpu::Device,
        queue: wgpu::Queue,
        node_buf: wgpu::Buffer,
        leaf_buf: wgpu::Buffer,
        bounds_buf: wgpu::Buffer,
        albedo_mode: AlbedoMode,
        has_transparency: bool,
        leaf_color_base_buf: wgpu::Buffer,
        leaf_color_chunks: Vec<wgpu::Buffer>,
        color_dummy_buf: wgpu::Buffer,
        per_chunk: u32,
        color_editable: bool,
        leaf_mat_buf: wgpu::Buffer,
        material_table_buf: wgpu::Buffer,
        max_binding: u64,
    ) -> Self {
        let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: std::mem::size_of::<GpuCamera>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let gtao_params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gtao params"),
            size: std::mem::size_of::<GtaoParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &gtao_params_buf,
            0,
            bytemuck::bytes_of(&GtaoParams::default()),
        );
        let prev_camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("prev camera"),
            size: std::mem::size_of::<GpuCamera>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let taa_params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("taa params"),
            size: std::mem::size_of::<TaaParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // 8 f32s: pos.xyz, radius, normal.xyz, active. Zero-initialized by
        // WebGPU ⇒ inactive by default (the byte-identical pin).
        let cursor_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hover cursor"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // The original sky gradient as {top, bottom} endpoints (it was linear
        // in t per channel, so two endpoints reproduce it exactly).
        let sky_default: [f32; 8] = [0.08, 0.10, 0.16, 1.0, 0.0, 0.0, 0.28, 1.0];
        let sky_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sky"),
            contents: bytemuck::cast_slice(&sky_default),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        // Sun direction (vec4: xyz + pad), shared by the shadow ray + direct light.
        let sun_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sun dir"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Pass 1: G-buffer (traverse → depth + normal). Concatenated after the
        // shared traversal core, so the structure bindings + `traverse_ray` exist.
        let gbuffer_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gbuffer"),
            source: wgpu::ShaderSource::Wgsl(
                // Truecolor g-buffer: injected PER_CHUNK + the shared traversal core
                // + the shared colour-lookup bodies + the g-buffer entry (which
                // declares the albedo target @7 and the colour buffers @8..11).
                format!(
                    "const PER_CHUNK: u32 = {}u;\n{}\n{}\n{}",
                    per_chunk,
                    include_str!("../shaders/traversal.wgsl"),
                    include_str!("../shaders/color_lookup.wgsl"),
                    include_str!("../shaders/gbuffer.wgsl"),
                )
                .into(),
            ),
        });
        let gbuffer_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gbuffer layout"),
            entries: &[
                buffers::storage_entry(0, true),     // nodes
                buffers::storage_entry(1, true),     // leaf_words
                buffers::storage_entry(2, true),     // leaf_bounds
                buffers::uniform_entry(3),           // camera
                storage_tex_entry(4, DEPTH_FORMAT),  // depth out
                storage_tex_entry(5, NORMAL_FORMAT), // normal out
                buffers::uniform_entry(6),           // sun dir
                storage_tex_entry(7, ALBEDO_FORMAT), // truecolor albedo out
                buffers::storage_entry(8, true),     // leaf_color_base
                buffers::storage_entry(9, true),     // leaf_color_0
                buffers::storage_entry(10, true),    // leaf_color_1
                buffers::storage_entry(11, true),    // leaf_color_2
            ],
        });
        let gbuffer_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gbuffer pl"),
            bind_group_layouts: &[Some(&gbuffer_layout)],
            immediate_size: 0,
        });
        let gbuffer_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gbuffer pipeline"),
            layout: Some(&gbuffer_pl),
            module: &gbuffer_shader,
            entry_point: Some("gbuffer_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // Pass 1 (skip-transparent variant, Stage C): same truecolor colour lookup +
        // the SAME g-buffer layout, but `gbuffer_opaque_main` traverses with
        // `traverse_ray_opaque` — skipping transparent voxels to capture the first
        // OPAQUE voxel as the backdrop (so GTAO/shadows light it and the blend pass
        // composites the glass in front). Selected at record time iff has_transparency;
        // built unconditionally so `gbuffer_opaque.wgsl` naga-validates every run.
        let gbuffer_skip_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gbuffer (skip-transparent)"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "const PER_CHUNK: u32 = {}u;\n{}\n{}\n{}\n{}",
                    buffers::COLOR_PER_CHUNK,
                    include_str!("../shaders/traversal.wgsl"),
                    include_str!("../shaders/color_lookup.wgsl"),
                    include_str!("../shaders/gbuffer.wgsl"),
                    include_str!("../shaders/gbuffer_opaque.wgsl"),
                )
                .into(),
            ),
        });
        let gbuffer_skip_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("gbuffer pipeline (skip-transparent)"),
                layout: Some(&gbuffer_pl),
                module: &gbuffer_skip_shader,
                entry_point: Some("gbuffer_opaque_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });

        // Pass 1 (palette variant): the same g-buffer entry, but the palette colour
        // lookup (leaf_mat@8 + material_table@9, 5/8 SSBO) instead of the truecolor
        // chunks. The MAT_* consts are injected from the `voxel_core::palette` layout
        // (drift-pinned). Built unconditionally so it naga-validates every run;
        // recorded only for `AlbedoMode::Palette` scenes.
        let palette_gbuffer_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gbuffer (palette)"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "const MAT_STRIDE_W: u32 = {}u;\nconst MAT_PAL_OFF: u32 = {}u;\nconst MAT_IDX_OFF: u32 = {}u;\n{}\n{}\n{}",
                    voxel_core::palette::STRIDE_W,
                    voxel_core::palette::PAL_OFF,
                    voxel_core::palette::IDX_OFF,
                    include_str!("../shaders/traversal.wgsl"),
                    include_str!("../shaders/palette_lookup.wgsl"),
                    include_str!("../shaders/gbuffer.wgsl"),
                )
                .into(),
            ),
        });
        let palette_gbuffer_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("gbuffer layout (palette)"),
                entries: &[
                    buffers::storage_entry(0, true),     // nodes
                    buffers::storage_entry(1, true),     // leaf_words
                    buffers::storage_entry(2, true),     // leaf_bounds
                    buffers::uniform_entry(3),           // camera
                    storage_tex_entry(4, DEPTH_FORMAT),  // depth out
                    storage_tex_entry(5, NORMAL_FORMAT), // normal out
                    buffers::uniform_entry(6),           // sun dir
                    storage_tex_entry(7, ALBEDO_FORMAT), // albedo out
                    buffers::storage_entry(8, true),     // leaf_mat
                    buffers::storage_entry(9, true),     // material_table
                ],
            });
        let palette_gbuffer_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gbuffer pl (palette)"),
            bind_group_layouts: &[Some(&palette_gbuffer_layout)],
            immediate_size: 0,
        });
        let palette_gbuffer_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("gbuffer pipeline (palette)"),
                layout: Some(&palette_gbuffer_pl),
                module: &palette_gbuffer_shader,
                entry_point: Some("gbuffer_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });

        // Pass 2: GTAO (read depth + normal → AO). Standalone module.
        let gtao_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gtao"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/gtao.wgsl").into()),
        });
        let gtao_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gtao layout"),
            entries: &[
                buffers::uniform_entry(0),       // camera
                sampled_tex_entry(1),            // depth in
                sampled_tex_entry(2),            // normal in
                storage_tex_entry(3, AO_FORMAT), // ao out
                buffers::uniform_entry(4),       // gtao params
                storage_tex_entry(5, AO_FORMAT), // edges out
            ],
        });
        let gtao_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gtao pl"),
            bind_group_layouts: &[Some(&gtao_layout)],
            immediate_size: 0,
        });
        let gtao_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gtao pipeline"),
            layout: Some(&gtao_pl),
            module: &gtao_shader,
            entry_point: Some("gtao_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // Passes 3+4: edge-aware bilateral denoise (sharp then soft). One shader,
        // two dispatches with different blur strengths via per-pass uniforms.
        let denoise_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gtao denoise"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/gtao_denoise.wgsl").into()),
        });
        let denoise_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("denoise layout"),
            entries: &[
                sampled_tex_entry(0),            // src ao
                sampled_tex_entry(1),            // src edges
                storage_tex_entry(2, AO_FORMAT), // dst ao
                buffers::uniform_entry(3),       // denoise params
            ],
        });
        let denoise_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("denoise pl"),
            bind_group_layouts: &[Some(&denoise_layout)],
            immediate_size: 0,
        });
        let denoise_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("denoise pipeline"),
            layout: Some(&denoise_pl),
            module: &denoise_shader,
            entry_point: Some("denoise_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let make_params = |label: &str| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: std::mem::size_of::<DenoiseParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let denoise_params1_buf = make_params("denoise params 1");
        let denoise_params2_buf = make_params("denoise params 2");

        // Pass 5: composite (read G-buffer + AO → shade → HDR colour). Standalone.
        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("composite"),
            // The themed-sky snippet (uniform @10 + `sky_color`) ahead of the
            // entry, so miss pixels paint the same gradient the forward path did.
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    buffers::SKY_WGSL,
                    include_str!("../shaders/composite.wgsl")
                )
                .into(),
            ),
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("composite layout"),
            entries: &[
                buffers::uniform_entry(0),          // camera
                sampled_tex_entry(1),               // depth in
                sampled_tex_entry(2),               // normal in
                sampled_tex_entry(3),               // ao in
                storage_tex_entry(4, COLOR_FORMAT), // colour out (pre-TAA)
                buffers::uniform_entry(5),          // sun dir
                sampled_tex_entry(6),               // per-voxel albedo
                buffers::uniform_entry(10),         // themed sky gradient
            ],
        });
        let composite_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("composite pl"),
            bind_group_layouts: &[Some(&composite_layout)],
            immediate_size: 0,
        });
        let composite_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("composite pipeline"),
            layout: Some(&composite_pl),
            module: &composite_shader,
            entry_point: Some("composite_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // Pass 6: TAA (reproject + variance-clip + accumulate → screen + history).
        let taa_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gtao taa"),
            // The hover-cursor snippet (uniform @9 + `cursor_tint`) ahead of the
            // entry: TAA is the final pass, so the ring tints the screen store
            // there (history stays clean — a moving ring never ghosts).
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    buffers::CURSOR_WGSL,
                    include_str!("../shaders/gtao_taa.wgsl")
                )
                .into(),
            ),
        });
        let taa_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("taa layout"),
            entries: &[
                buffers::uniform_entry(0),           // camera
                buffers::uniform_entry(1),           // prev camera
                sampled_tex_entry(2),                // colour current
                sampled_tex_entry(3),                // colour history (read)
                sampled_tex_entry(4),                // depth current
                sampled_tex_entry(5),                // prev depth
                storage_tex_entry(6, OUTPUT_FORMAT), // out colour (screen)
                storage_tex_entry(7, COLOR_FORMAT),  // out history (write)
                buffers::uniform_entry(8),           // taa params
                buffers::uniform_entry(9),           // hover cursor (zero = inactive)
            ],
        });
        let taa_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("taa pl"),
            bind_group_layouts: &[Some(&taa_layout)],
            immediate_size: 0,
        });
        let taa_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("taa pipeline"),
            layout: Some(&taa_pl),
            module: &taa_shader,
            entry_point: Some("taa_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // Pass 7 (Stage C): forward-blend transparency — composites transparent voxels
        // over the lit composite result into `color_blended`, before TAA. Built
        // unconditionally so it always naga-validates; dispatched only for scenes with
        // `has_transparency`. Reuses the truecolor colour buffers @5..8.
        let blend_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blend forward"),
            source: wgpu::ShaderSource::Wgsl(
                buffers::blend_forward_shader_source(per_chunk, buffers::MAX_BLEND).into(),
            ),
        });
        let blend_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blend layout"),
            entries: &[
                buffers::storage_entry(0, true),     // nodes
                buffers::storage_entry(1, true),     // leaf_words
                buffers::storage_entry(2, true),     // leaf_bounds
                buffers::uniform_entry(3),           // camera
                buffers::storage_entry(5, true),     // leaf_color_base
                buffers::storage_entry(6, true),     // leaf_color_0
                buffers::storage_entry(7, true),     // leaf_color_1
                buffers::storage_entry(8, true),     // leaf_color_2
                sampled_tex_entry(9),                // lit_color (composite output)
                sampled_tex_entry(10),               // opaque depth
                storage_tex_entry(11, COLOR_FORMAT), // blended out
            ],
        });
        let blend_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blend pl"),
            bind_group_layouts: &[Some(&blend_layout)],
            immediate_size: 0,
        });
        let blend_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("blend pipeline"),
            layout: Some(&blend_pl),
            module: &blend_shader,
            entry_point: Some("blend_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let timing = ctx.supports_timestamps().then(|| {
            // 2 timestamps (begin + end) per timed stage.
            let ts_count = u32::try_from(2 * NUM_TIMED_STAGES).expect("stage count fits u32");
            let ts_bytes = u64::from(ts_count) * 8;
            let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("render timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: ts_count,
            });
            let resolve = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("render ts resolve"),
                size: ts_bytes,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("render ts readback"),
                size: ts_bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            RenderTiming {
                query_set,
                resolve,
                readback,
                period: queue.get_timestamp_period(),
            }
        });

        Self {
            device,
            queue,
            gbuffer_pipeline,
            gbuffer_layout,
            gbuffer_skip_pipeline,
            palette_gbuffer_pipeline,
            palette_gbuffer_layout,
            gtao_pipeline,
            gtao_layout,
            denoise_pipeline,
            denoise_layout,
            denoise_params1_buf,
            denoise_params2_buf,
            composite_pipeline,
            composite_layout,
            taa_pipeline,
            taa_layout,
            blend_pipeline,
            blend_layout,
            gbuf: None,
            node_buf,
            leaf_buf,
            bounds_buf,
            leaf_color_base_buf,
            leaf_color_chunks,
            color_dummy_buf,
            per_chunk,
            color_editable,
            leaf_mat_buf,
            material_table_buf,
            albedo_mode,
            has_transparency,
            camera_buf,
            prev_camera_buf,
            gtao_params_buf,
            taa_params_buf,
            gtao_quality: GtaoParams::default(),
            prev_camera: GpuCamera::zeroed(),
            frame_index: 0,
            history_valid: false,
            gtao_enabled: true,
            denoise: true,
            shadows: true,
            coarse_shadows: false,
            cursor_buf,
            sky_buf,
            sun_buf,
            // Posed near the original fixed sun (≈azimuth of (0.5,_,0.28)).
            sun_phase: 0.51,
            max_binding,
            timing,
        }
    }

    /// Sets the GTAO quality preset (slice × step counts) and effect radius.
    /// The per-frame `frame_index` is overwritten by the renderer. Takes effect
    /// on the next render.
    pub fn set_gtao_params(&mut self, params: GtaoParams) {
        self.gtao_quality = params;
    }

    /// Enables/disables noise reduction (debug). When off, composite reads the raw
    /// GTAO AO and the TAA passes through (no spatial denoise, no temporal
    /// accumulation). Takes effect on the next render.
    pub fn set_denoise(&mut self, on: bool) {
        self.denoise = on;
    }

    /// Enables/disables the GTAO term entirely. Off skips the GTAO + denoise
    /// dispatches (their whole cost) and composite shades with full ambient
    /// visibility. Takes effect on the next render.
    pub fn set_gtao(&mut self, on: bool) {
        self.gtao_enabled = on;
    }

    /// Sets the sky gradient (theme changes): a vertical ramp from `top_rgba`
    /// to `bottom_rgba` (sRGB RGBA8, R low), dithered per pixel on the GPU so
    /// the subtle ramp never bands. One 32-byte uniform write.
    pub fn set_sky(&self, top_rgba: u32, bottom_rgba: u32) {
        #[allow(clippy::cast_precision_loss)] // a masked byte is exact in f32
        let ch = |v: u32, shift: u32| ((v >> shift) & 0xff) as f32 / 255.0;
        let unpack = |v: u32| [ch(v, 0), ch(v, 8), ch(v, 16), ch(v, 24)];
        let data: [f32; 8] = {
            let (t, b) = (unpack(top_rgba), unpack(bottom_rgba));
            [t[0], t[1], t[2], t[3], b[0], b[1], b[2], b[3]]
        };
        self.queue
            .write_buffer(&self.sky_buf, 0, bytemuck::cast_slice(&data));
    }

    /// Positions the hover-cursor ring (brush-editing Stage D): the highlight
    /// where the brush sphere of `radius` voxels centred at `pos` (world voxel
    /// space) intersects the surface. `active = false` zeroes the flag — the
    /// default state, in which every render is byte-identical to a cursor-free
    /// build. One 32-byte uniform write; drawn by the TAA (final) pass so the
    /// ring never enters the temporal history.
    pub fn set_cursor(&self, pos: [f32; 3], radius: f32, active: bool) {
        let data: [f32; 8] = [
            pos[0],
            pos[1],
            pos[2],
            radius,
            0.0,
            0.0,
            0.0,
            if active { 1.0 } else { 0.0 },
        ];
        self.queue
            .write_buffer(&self.cursor_buf, 0, bytemuck::cast_slice(&data));
    }

    /// Enables/disables the ray-traced sun shadow. When off (the web default),
    /// the G-buffer skips the shadow ray and everything reads as lit — removes
    /// the sharp umbras and their cost. Takes effect on the next render.
    pub fn set_shadows(&mut self, on: bool) {
        self.shadows = on;
    }

    /// Selects the coarse brick-level shadow trace (`traverse_occluded`) over the
    /// exact per-voxel one — cheaper, slightly blockier. Takes effect next render.
    pub fn set_coarse_shadows(&mut self, on: bool) {
        self.coarse_shadows = on;
    }

    /// Sets the sun azimuth (radians) — the static sun's pose knob. Takes effect
    /// on the next render.
    pub fn set_sun_phase(&mut self, phase: f32) {
        self.sun_phase = phase;
    }

    /// Ensures the viewport-sized G-buffer textures exist and match `(w, h)`,
    /// recreating them on a size change. Returns a reference to them.
    fn ensure_gbuf(&mut self, w: u32, h: u32) -> &GBuf {
        let stale = self
            .gbuf
            .as_ref()
            .is_none_or(|g| g.width != w || g.height != h);
        if stale {
            let base = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
            let make = |label: &str, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
                let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                });
                let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
                (tex, view)
            };
            // depth is COPY_SRC (captured into prev_depth each frame for the TAA).
            let (depth, depth_view) = make(
                "gbuf depth",
                DEPTH_FORMAT,
                base | wgpu::TextureUsages::COPY_SRC,
            );
            let (normal, normal_view) = make("gbuf normal", NORMAL_FORMAT, base);
            let (albedo, albedo_view) = make("gbuf albedo", ALBEDO_FORMAT, base);
            let (ao, ao_view) = make("gtao ao", AO_FORMAT, base);
            let (edges, edges_view) = make("gtao edges", AO_FORMAT, base);
            let (ao_pong, ao_pong_view) = make("gtao ao pong", AO_FORMAT, base);
            let (ao_denoised, ao_denoised_view) = make("gtao ao denoised", AO_FORMAT, base);
            let (color, color_view) = make("composite color", COLOR_FORMAT, base);
            let (color_blended, color_blended_view) = make("blended color", COLOR_FORMAT, base);
            let (h0, h0v) = make("taa history 0", COLOR_FORMAT, base);
            let (h1, h1v) = make("taa history 1", COLOR_FORMAT, base);
            let (prev_depth, prev_depth_view) = make(
                "prev depth",
                DEPTH_FORMAT,
                base | wgpu::TextureUsages::COPY_DST,
            );
            self.gbuf = Some(GBuf {
                width: w,
                height: h,
                depth,
                depth_view,
                normal,
                normal_view,
                albedo,
                albedo_view,
                ao,
                ao_view,
                edges,
                edges_view,
                ao_pong,
                ao_pong_view,
                ao_denoised,
                ao_denoised_view,
                color,
                color_view,
                color_blended,
                color_blended_view,
                history: [h0, h1],
                history_view: [h0v, h1v],
                prev_depth,
                prev_depth_view,
            });
            self.history_valid = false; // freshly created → no prior frame to reproject
        }
        self.gbuf.as_ref().expect("just created")
    }

    /// Patches a single leaf onto the GPU after an in-place [`Edit::Leaf`].
    ///
    /// `structure` must already have had [`SchoolBBuffer::patch_leaf`] applied
    /// for `leaf_idx`; this copies that one leaf's 16 occupancy words (64 bytes
    /// at `leaf_idx * 64`) and its packed bounds word (4 bytes at `leaf_idx * 4`)
    /// into the resident buffers via `queue.write_buffer`. That is an `O(1)`
    /// upload flushed by the next queue submission (the next rendered frame),
    /// instead of rebuilding the whole structure. To force it through
    /// synchronously — e.g. when timing an edit headlessly — call
    /// [`flush_and_wait`](Self::flush_and_wait).
    ///
    /// [`Edit::Leaf`]: voxel_core::Edit::Leaf
    /// Works for the palette path and the **paged** (editable) truecolor path —
    /// both keep `leaf_buf`/`bounds_buf` `COPY_DST` — but the caller must also sync
    /// the touched leaf's colour page on a paged scene (via
    /// [`update_color_page`](Self::update_color_page)). Small in-place edits are
    /// absorbed by the TAA's variance clip, so history is deliberately NOT
    /// invalidated here (a per-stroke reset would shimmer under the brush).
    ///
    /// # Errors
    /// Returns [`GpuError::Unsupported`] on a **static** truecolor renderer:
    /// per-voxel colour is build-once there, so the scene must be re-baked via
    /// [`new`](Self::new).
    pub fn update_leaf(&self, structure: &SchoolBBuffer, leaf_idx: u32) -> Result<(), GpuError> {
        if self.albedo_mode == AlbedoMode::Truecolor && !self.color_editable {
            return Err(GpuError::Unsupported {
                n: structure.resolution().voxels_per_axis(),
                reason: "static truecolor renderer is build-once; re-bake via GpuRenderer::new after an edit",
            });
        }
        let words = structure.leaf_at(leaf_idx).words32();
        self.queue.write_buffer(
            &self.leaf_buf,
            u64::from(leaf_idx) * 64,
            bytemuck::cast_slice(&words),
        );
        let bounds = structure.leaf_bounds_words()[leaf_idx as usize];
        self.queue.write_buffer(
            &self.bounds_buf,
            u64::from(leaf_idx) * 4,
            bytemuck::bytes_of(&bounds),
        );
        Ok(())
    }

    /// Patches a single leaf's material slot onto the GPU after an in-place
    /// material edit — the same O(1) shape as [`update_leaf`](Self::update_leaf).
    /// A *spilled* edit bumps the topology generation and must go through
    /// [`reupload`](Self::reupload) instead.
    ///
    /// # Errors
    /// Returns [`GpuError::Unsupported`] on a non-palette renderer (no `leaf_mat`
    /// buffer is bound there).
    pub fn update_leaf_mat(
        &self,
        structure: &SchoolBBuffer,
        leaf_idx: u32,
    ) -> Result<(), GpuError> {
        if self.albedo_mode != AlbedoMode::Palette {
            return Err(GpuError::Unsupported {
                n: structure.resolution().voxels_per_axis(),
                reason: "truecolor renderer is build-once; re-bake via GpuRenderer::new after an edit",
            });
        }
        let stride_w = voxel_core::palette::STRIDE_W;
        let base = leaf_idx as usize * stride_w;
        let slot = &structure.leaf_mat_words()[base..base + stride_w];
        self.queue.write_buffer(
            &self.leaf_mat_buf,
            (base * 4) as u64,
            bytemuck::cast_slice(slot),
        );
        Ok(())
    }

    /// The paged (editable) truecolor `per_chunk` if this is such a renderer.
    fn paged_per_chunk(&self) -> Option<u32> {
        self.color_editable.then_some(self.per_chunk)
    }

    /// Writes one leaf's colour page (`words`, padded to its class capacity) into
    /// the pool at `offset_entries`, growing/adding the target chunk if the write
    /// runs past its current size. Paired with an occupancy patch this is the
    /// in-place edit path; a topology edit goes through
    /// [`reupload_paged`](Self::reupload_paged) plus the pages it touched. Paged
    /// (editable) truecolor only.
    ///
    /// # Errors
    /// [`GpuError::Unsupported`] on a palette or static-truecolor renderer.
    pub fn update_color_page(
        &mut self,
        offset_entries: u64,
        words: &[u32],
    ) -> Result<(), GpuError> {
        let per_chunk = self.paged_per_chunk().ok_or(GpuError::Unsupported {
            n: 0,
            reason: "update_color_page requires a paged (editable) truecolor renderer",
        })?;
        let per = u64::from(per_chunk);
        let chunk = offset_entries / per;
        let local = offset_entries % per;
        let end = local + words.len() as u64;
        debug_assert!(
            end <= per,
            "a colour page must not straddle a chunk boundary"
        );
        self.grow_chunk(
            u32::try_from(chunk).expect("chunk index fits u32"),
            u32::try_from(end).expect("chunk-local end fits u32"),
        )?;
        let chunk = usize::try_from(chunk).expect("chunk index fits usize");
        self.queue.write_buffer(
            &self.leaf_color_chunks[chunk],
            local * 4,
            bytemuck::cast_slice(words),
        );
        Ok(())
    }

    /// Rewrites leaf `leaf_idx`'s page-table entry to `page_offset` after an edit
    /// moved its page (a class change on an in-place occupancy edit). Paired with
    /// an [`update_color_page`](Self::update_color_page) of the new page's
    /// contents. Paged (editable) truecolor only.
    ///
    /// # Errors
    /// [`GpuError::Unsupported`] on a palette or static-truecolor renderer.
    pub fn update_page_word(&self, leaf_idx: u32, page_offset: u32) -> Result<(), GpuError> {
        if !self.color_editable {
            return Err(GpuError::Unsupported {
                n: 0,
                reason: "update_page_word requires a paged (editable) truecolor renderer",
            });
        }
        self.queue.write_buffer(
            &self.leaf_color_base_buf,
            u64::from(leaf_idx) * 4,
            bytemuck::bytes_of(&page_offset),
        );
        Ok(())
    }

    /// Ensures pool `chunk` exists and holds at least `new_entries` entries,
    /// adding any missing lower chunks (each a full `per_chunk`) and growing the
    /// target to a full `per_chunk` via `copy_buffer_to_buffer` (one-time per
    /// chunk). Paged (editable) truecolor only.
    ///
    /// # Errors
    /// [`GpuError::Unsupported`] on a palette or static-truecolor renderer.
    pub fn grow_chunk(&mut self, chunk: u32, new_entries: u32) -> Result<(), GpuError> {
        let per_chunk = self.paged_per_chunk().ok_or(GpuError::Unsupported {
            n: 0,
            reason: "grow_chunk requires a paged (editable) truecolor renderer",
        })?;
        let full_bytes = u64::from(per_chunk) * 4;
        let want_bytes = u64::from(new_entries) * 4;
        let chunk = chunk as usize;
        while self.leaf_color_chunks.len() <= chunk {
            self.leaf_color_chunks
                .push(self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("leaf_color_pool_grow"),
                    size: full_bytes,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_DST
                        | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }));
        }
        if self.leaf_color_chunks[chunk].size() < want_bytes {
            let grown = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("leaf_color_pool_grow"),
                size: full_bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let old_size = self.leaf_color_chunks[chunk].size();
            let mut enc = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("grow chunk copy"),
                });
            enc.copy_buffer_to_buffer(&self.leaf_color_chunks[chunk], 0, &grown, 0, old_size);
            self.queue.submit(std::iter::once(enc.finish()));
            self.leaf_color_chunks[chunk] = grown;
        }
        Ok(())
    }

    /// Replaces the resident structure and page table after a topology edit on a
    /// paged truecolor scene — the paged analogue of [`reupload`](Self::reupload).
    /// Rebuilds nodes/leaves/bounds and the `leaf_color_page` buffer from
    /// `structure` (a fresh [`SchoolBBuffer::from_sparse`]); the colour **pool is
    /// untouched**, so the ~hundreds-of-MB colour data is never re-uploaded. The
    /// caller re-uploads only the pages the edit actually changed via
    /// [`update_color_page`](Self::update_color_page). Invalidates the TAA
    /// history (a wholesale structure swap is too big for the variance clip).
    ///
    /// # Errors
    /// [`GpuError::BufferTooLarge`] if a structure buffer exceeds the binding cap,
    /// or [`GpuError::Unsupported`] on a palette or static-truecolor renderer.
    pub fn reupload_paged(&mut self, structure: &SchoolBBuffer) -> Result<(), GpuError> {
        if self.paged_per_chunk().is_none() {
            return Err(GpuError::Unsupported {
                n: structure.resolution().voxels_per_axis(),
                reason: "reupload_paged requires a paged (editable) truecolor renderer",
            });
        }
        let (node_buf, leaf_buf, bounds_buf) =
            buffers::upload_structure(&self.device, structure, self.max_binding)?;
        self.node_buf = node_buf;
        self.leaf_buf = leaf_buf;
        self.bounds_buf = bounds_buf;
        let mut page_words = structure.leaf_color_page_words().to_vec();
        if page_words.is_empty() {
            page_words = vec![0u32];
        }
        self.leaf_color_base_buf = wgpu::util::DeviceExt::create_buffer_init(
            &self.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("leaf_color_page"),
                contents: bytemuck::cast_slice(&page_words),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            },
        );
        self.history_valid = false;
        Ok(())
    }

    /// Replaces the resident structure after a topology edit
    /// ([`Edit::Topology`]), which renumbers leaf indices and invalidates the
    /// node buffer's `subtree_base` offsets. Rebuilds all three buffers from
    /// `structure` (a fresh [`SchoolBBuffer::from_sparse`] of the edited tree)
    /// and swaps them in; the per-frame bind group picks them up on the next
    /// render.
    ///
    /// [`Edit::Topology`]: voxel_core::Edit::Topology
    /// # Errors
    /// Returns [`GpuError::BufferTooLarge`] if a structure buffer exceeds the
    /// binding cap, or [`GpuError::Unsupported`] on a truecolor renderer (colour
    /// is build-once there — a topology edit renumbers leaves and invalidates the
    /// colour chunks; static scenes re-bake via [`new`](Self::new), paged ones go
    /// through [`reupload_paged`](Self::reupload_paged)).
    pub fn reupload(&mut self, structure: &SchoolBBuffer) -> Result<(), GpuError> {
        if self.albedo_mode == AlbedoMode::Truecolor {
            return Err(GpuError::Unsupported {
                n: structure.resolution().voxels_per_axis(),
                reason: "truecolor renderer is build-once; re-bake via GpuRenderer::new after an edit",
            });
        }
        let (node_buf, leaf_buf, bounds_buf) =
            buffers::upload_structure(&self.device, structure, self.max_binding)?;
        self.node_buf = node_buf;
        self.leaf_buf = leaf_buf;
        self.bounds_buf = bounds_buf;
        // The material slots are index-parallel with the leaves, so a topology
        // edit invalidates them too — rebuild `leaf_mat` for palette scenes. The
        // global colour table is unchanged by a topology edit, so it stays.
        if self.albedo_mode == AlbedoMode::Palette {
            let stride_w = voxel_core::palette::STRIDE_W;
            let mut mat_words = structure.leaf_mat_words().to_vec();
            if mat_words.is_empty() {
                mat_words = vec![0u32; stride_w];
            }
            self.leaf_mat_buf = wgpu::util::DeviceExt::create_buffer_init(
                &self.device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("leaf_mat"),
                    contents: bytemuck::cast_slice(&mat_words),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                },
            );
        }
        self.history_valid = false;
        Ok(())
    }

    /// Forces any staged buffer writes (from [`update_leaf`](Self::update_leaf))
    /// through to the GPU and blocks until the device is idle. The render loop
    /// does not need this — the next frame's submit flushes staged writes — but a
    /// headless caller timing an edit's full round-trip can use it.
    pub fn flush_and_wait(&self) -> Result<(), GpuError> {
        let encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("flush"),
            });
        self.queue.submit(std::iter::once(encoder.finish()));
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|_| GpuError::Poll)?;
        Ok(())
    }

    /// Whether this renderer can report a GPU-timeline kernel time (i.e. the
    /// device supports compute-pass timestamp queries).
    pub fn supports_timing(&self) -> bool {
        self.timing.is_some()
    }

    /// Records the render compute pass into `encoder`, writing the shaded image
    /// to `output` (an [`OUTPUT_FORMAT`] storage-texture view of size
    /// `width × height`). The caller blits `output` to its surface.
    pub fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        camera: &GpuCamera,
        output: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        self.record(encoder, camera, output, width, height, false);
    }

    /// Like [`render`](Self::render), but brackets the compute pass with
    /// timestamp queries and appends their resolve+copy to `encoder` (when the
    /// device supports timestamps). After the caller submits `encoder` and the
    /// GPU completes, [`last_kernel_ns`](Self::last_kernel_ns) returns the
    /// traverse+shade time in nanoseconds. With no timestamp support this is
    /// identical to [`render`](Self::render) and `last_kernel_ns` yields `None`.
    pub fn render_timed(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        camera: &GpuCamera,
        output: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        self.record(encoder, camera, output, width, height, true);
    }

    /// Shared multi-pass recorder: G-buffer → GTAO → denoise×2 → composite. When
    /// `timed`, the GTAO pass alone is bracketed with timestamp queries.
    #[allow(clippy::too_many_lines)] // straight-line: 5 bind groups + 5 dispatches
    fn record(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        camera: &GpuCamera,
        output: &wgpu::TextureView,
        width: u32,
        height: u32,
        timed: bool,
    ) {
        // Fold the render flags into dims.w: bit0 = shadows enabled, bit1 =
        // coarse (brick-level) shadow trace, bit2 = palette albedo, bit7 =
        // truecolor albedo. The caller's camera is copied, never mutated.
        let mut camera = *camera;
        camera.dims[3] = u32::from(self.shadows)
            | (u32::from(self.coarse_shadows) << 1)
            | (u32::from(!self.gtao_enabled) << 3) // AO disabled → composite reads 1.0
            | (u32::from(self.albedo_mode == AlbedoMode::Truecolor) << 7) // truecolor albedo
            | (u32::from(self.albedo_mode == AlbedoMode::Palette) << 2); // palette albedo
        self.queue
            .write_buffer(&self.camera_buf, 0, bytemuck::bytes_of(&camera));

        // The static sun-direction uniform: azimuth from `sun_phase`, a fixed mid
        // elevation so shadows stay long enough to read. `.w` is unused padding.
        let (sa, ca) = self.sun_phase.sin_cos();
        let d = [ca, SUN_ELEVATION, sa];
        let inv = 1.0 / (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        self.queue.write_buffer(
            &self.sun_buf,
            0,
            bytemuck::cast_slice(&[d[0] * inv, d[1] * inv, d[2] * inv, 0.0_f32]),
        );
        self.queue.write_buffer(
            &self.denoise_params1_buf,
            0,
            bytemuck::bytes_of(&DenoiseParams {
                blur_beta: DENOISE_BLUR_BETA / 5.0,
                final_apply: 0,
                width,
                height,
            }),
        );
        self.queue.write_buffer(
            &self.denoise_params2_buf,
            0,
            bytemuck::bytes_of(&DenoiseParams {
                blur_beta: DENOISE_BLUR_BETA,
                final_apply: 1,
                width,
                height,
            }),
        );
        self.ensure_gbuf(width, height);

        // Per-frame GTAO noise rotation + TAA reprojection uniforms.
        self.queue.write_buffer(
            &self.gtao_params_buf,
            0,
            bytemuck::bytes_of(&GtaoParams {
                frame_index: self.frame_index,
                ..self.gtao_quality
            }),
        );
        self.queue.write_buffer(
            &self.prev_camera_buf,
            0,
            bytemuck::bytes_of(&self.prev_camera),
        );
        self.queue.write_buffer(
            &self.taa_params_buf,
            0,
            bytemuck::bytes_of(&TaaParams {
                width,
                height,
                frame_index: self.frame_index,
                // Denoise off → force TAA passthrough (no temporal accumulation).
                history_valid: u32::from(self.history_valid && self.denoise),
            }),
        );

        // Ping-pong the colour history: read last frame's, write this frame's.
        let read = (self.frame_index % 2) as usize;
        let write = ((self.frame_index + 1) % 2) as usize;
        let gbuf = self.gbuf.as_ref().expect("ensured");
        // Select the g-buffer variant by albedo mode: palette binds leaf_mat@8 +
        // material_table@9 (5/8 SSBO); truecolor (and the None/occupancy fallback)
        // binds colour base@8 + 3 chunk slots@9..11 (7/8 SSBO). Both write the shared
        // gbuf_albedo@7; composite reads it gated by dims.w (bit7 truecolor/bit2 palette).
        let palette = self.albedo_mode == AlbedoMode::Palette;
        let gbuffer_pipeline = if palette {
            &self.palette_gbuffer_pipeline
        } else if self.has_transparency {
            // Truecolor + transparency: skip transparent voxels to capture the OPAQUE
            // backdrop, so the blend pass composites the glass over the lit opaque.
            &self.gbuffer_skip_pipeline
        } else {
            &self.gbuffer_pipeline
        };
        let gbuffer_bind = if palette {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("gbuffer bind (palette)"),
                layout: &self.palette_gbuffer_layout,
                entries: &[
                    buffers::bind(0, self.node_buf.as_entire_binding()),
                    buffers::bind(1, self.leaf_buf.as_entire_binding()),
                    buffers::bind(2, self.bounds_buf.as_entire_binding()),
                    buffers::bind(3, self.camera_buf.as_entire_binding()),
                    buffers::bind(4, wgpu::BindingResource::TextureView(&gbuf.depth_view)),
                    buffers::bind(5, wgpu::BindingResource::TextureView(&gbuf.normal_view)),
                    buffers::bind(6, self.sun_buf.as_entire_binding()),
                    buffers::bind(7, wgpu::BindingResource::TextureView(&gbuf.albedo_view)),
                    buffers::bind(8, self.leaf_mat_buf.as_entire_binding()),
                    buffers::bind(9, self.material_table_buf.as_entire_binding()),
                ],
            })
        } else {
            // Colour-chunk slots @9..11: the real chunks (≤ N_MAX_CHUNKS) then the
            // shared 1-`u32` dummy for the unused tail (and for None/occupancy scenes).
            let cc0 = self
                .leaf_color_chunks
                .first()
                .unwrap_or(&self.color_dummy_buf);
            let cc1 = self
                .leaf_color_chunks
                .get(1)
                .unwrap_or(&self.color_dummy_buf);
            let cc2 = self
                .leaf_color_chunks
                .get(2)
                .unwrap_or(&self.color_dummy_buf);
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("gbuffer bind (truecolor)"),
                layout: &self.gbuffer_layout,
                entries: &[
                    buffers::bind(0, self.node_buf.as_entire_binding()),
                    buffers::bind(1, self.leaf_buf.as_entire_binding()),
                    buffers::bind(2, self.bounds_buf.as_entire_binding()),
                    buffers::bind(3, self.camera_buf.as_entire_binding()),
                    buffers::bind(4, wgpu::BindingResource::TextureView(&gbuf.depth_view)),
                    buffers::bind(5, wgpu::BindingResource::TextureView(&gbuf.normal_view)),
                    buffers::bind(6, self.sun_buf.as_entire_binding()),
                    buffers::bind(7, wgpu::BindingResource::TextureView(&gbuf.albedo_view)),
                    buffers::bind(8, self.leaf_color_base_buf.as_entire_binding()),
                    buffers::bind(9, cc0.as_entire_binding()),
                    buffers::bind(10, cc1.as_entire_binding()),
                    buffers::bind(11, cc2.as_entire_binding()),
                ],
            })
        };
        let gtao_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gtao bind"),
            layout: &self.gtao_layout,
            entries: &[
                buffers::bind(0, self.camera_buf.as_entire_binding()),
                buffers::bind(1, wgpu::BindingResource::TextureView(&gbuf.depth_view)),
                buffers::bind(2, wgpu::BindingResource::TextureView(&gbuf.normal_view)),
                buffers::bind(3, wgpu::BindingResource::TextureView(&gbuf.ao_view)),
                buffers::bind(4, self.gtao_params_buf.as_entire_binding()),
                buffers::bind(5, wgpu::BindingResource::TextureView(&gbuf.edges_view)),
            ],
        });
        // Denoise pass 1 (sharp): raw ao + edges → ao_pong.
        let denoise1_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("denoise1 bind"),
            layout: &self.denoise_layout,
            entries: &[
                buffers::bind(0, wgpu::BindingResource::TextureView(&gbuf.ao_view)),
                buffers::bind(1, wgpu::BindingResource::TextureView(&gbuf.edges_view)),
                buffers::bind(2, wgpu::BindingResource::TextureView(&gbuf.ao_pong_view)),
                buffers::bind(3, self.denoise_params1_buf.as_entire_binding()),
            ],
        });
        // Denoise pass 2 (soft): ao_pong + edges → ao_denoised.
        let denoise2_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("denoise2 bind"),
            layout: &self.denoise_layout,
            entries: &[
                buffers::bind(0, wgpu::BindingResource::TextureView(&gbuf.ao_pong_view)),
                buffers::bind(1, wgpu::BindingResource::TextureView(&gbuf.edges_view)),
                buffers::bind(
                    2,
                    wgpu::BindingResource::TextureView(&gbuf.ao_denoised_view),
                ),
                buffers::bind(3, self.denoise_params2_buf.as_entire_binding()),
            ],
        });
        // Composite writes the pre-TAA HDR colour buffer (not the screen).
        let composite_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("composite bind"),
            layout: &self.composite_layout,
            entries: &[
                buffers::bind(0, self.camera_buf.as_entire_binding()),
                buffers::bind(1, wgpu::BindingResource::TextureView(&gbuf.depth_view)),
                buffers::bind(2, wgpu::BindingResource::TextureView(&gbuf.normal_view)),
                // Denoise off → composite reads the raw GTAO AO instead of denoised.
                buffers::bind(
                    3,
                    wgpu::BindingResource::TextureView(if self.denoise {
                        &gbuf.ao_denoised_view
                    } else {
                        &gbuf.ao_view
                    }),
                ),
                buffers::bind(4, wgpu::BindingResource::TextureView(&gbuf.color_view)),
                buffers::bind(5, self.sun_buf.as_entire_binding()),
                buffers::bind(6, wgpu::BindingResource::TextureView(&gbuf.albedo_view)),
                buffers::bind(10, self.sky_buf.as_entire_binding()),
            ],
        });
        // TAA: reproject + clip + accumulate → screen `output` + next history.
        let taa_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("taa bind"),
            layout: &self.taa_layout,
            entries: &[
                buffers::bind(0, self.camera_buf.as_entire_binding()),
                buffers::bind(1, self.prev_camera_buf.as_entire_binding()),
                // TAA reads the blended result when transparents were composited this
                // frame, else the plain composite colour (Stage C repoint).
                buffers::bind(
                    2,
                    wgpu::BindingResource::TextureView(if self.has_transparency {
                        &gbuf.color_blended_view
                    } else {
                        &gbuf.color_view
                    }),
                ),
                buffers::bind(
                    3,
                    wgpu::BindingResource::TextureView(&gbuf.history_view[read]),
                ),
                buffers::bind(4, wgpu::BindingResource::TextureView(&gbuf.depth_view)),
                buffers::bind(5, wgpu::BindingResource::TextureView(&gbuf.prev_depth_view)),
                buffers::bind(6, wgpu::BindingResource::TextureView(output)),
                buffers::bind(
                    7,
                    wgpu::BindingResource::TextureView(&gbuf.history_view[write]),
                ),
                buffers::bind(8, self.taa_params_buf.as_entire_binding()),
                buffers::bind(9, self.cursor_buf.as_entire_binding()),
            ],
        });

        // Stage C: the forward-blend bind — built only when the scene has transparent
        // voxels (the colour buffers are the real truecolor ones, since
        // has_transparency ⟹ Truecolor). Reads the lit composite (`color`) + opaque
        // depth, writes `color_blended`.
        let blend_bind = if self.has_transparency {
            let bc0 = self
                .leaf_color_chunks
                .first()
                .unwrap_or(&self.color_dummy_buf);
            let bc1 = self
                .leaf_color_chunks
                .get(1)
                .unwrap_or(&self.color_dummy_buf);
            let bc2 = self
                .leaf_color_chunks
                .get(2)
                .unwrap_or(&self.color_dummy_buf);
            Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("blend bind"),
                layout: &self.blend_layout,
                entries: &[
                    buffers::bind(0, self.node_buf.as_entire_binding()),
                    buffers::bind(1, self.leaf_buf.as_entire_binding()),
                    buffers::bind(2, self.bounds_buf.as_entire_binding()),
                    buffers::bind(3, self.camera_buf.as_entire_binding()),
                    buffers::bind(5, self.leaf_color_base_buf.as_entire_binding()),
                    buffers::bind(6, bc0.as_entire_binding()),
                    buffers::bind(7, bc1.as_entire_binding()),
                    buffers::bind(8, bc2.as_entire_binding()),
                    buffers::bind(9, wgpu::BindingResource::TextureView(&gbuf.color_view)),
                    buffers::bind(10, wgpu::BindingResource::TextureView(&gbuf.depth_view)),
                    buffers::bind(
                        11,
                        wgpu::BindingResource::TextureView(&gbuf.color_blended_view),
                    ),
                ],
            }))
        } else {
            None
        };

        let (gx, gy) = (width.div_ceil(8), height.div_ceil(8));
        let timing = if timed { self.timing.as_ref() } else { None };
        // Per-stage GPU timing: each pass writes its own begin/end timestamp
        // (indices 2·stage, 2·stage+1) — the Metal-portable bracketing.
        let ts = |stage: u32| {
            timing.map(|t| wgpu::ComputePassTimestampWrites {
                query_set: &t.query_set,
                beginning_of_pass_write_index: Some(2 * stage),
                end_of_pass_write_index: Some(2 * stage + 1),
            })
        };

        {
            // Stage 0 GBUF — traverse → depth + normal + sun shadow ray.
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gbuffer pass"),
                timestamp_writes: ts(0),
            });
            pass.set_pipeline(gbuffer_pipeline);
            pass.set_bind_group(0, &gbuffer_bind, &[]);
            pass.dispatch_workgroups(gx, gy, 1);
        }
        {
            // Stage 1 GTAO — depth + normal → AO (cost varies by quality preset).
            // Recorded even when AO is off so the timestamp slot stays valid;
            // dispatched only when on (off = the whole cost vanishes and
            // composite reads full visibility via dims.w bit3).
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gtao pass"),
                timestamp_writes: ts(1),
            });
            if self.gtao_enabled {
                pass.set_pipeline(&self.gtao_pipeline);
                pass.set_bind_group(0, &gtao_bind, &[]);
                pass.dispatch_workgroups(gx, gy, 1);
            }
        }
        // Stages 2,3 DNS1/DNS2 — edge-aware bilateral denoise (sharp then soft).
        for (stage, bind) in [(2u32, &denoise1_bind), (3u32, &denoise2_bind)] {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("denoise pass"),
                timestamp_writes: ts(stage),
            });
            if self.gtao_enabled {
                pass.set_pipeline(&self.denoise_pipeline);
                pass.set_bind_group(0, bind, &[]);
                pass.dispatch_workgroups(gx, gy, 1);
            }
        }
        {
            // Stage 4 COMP — read G-buffer + denoised AO → shade → HDR colour.
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("composite pass"),
                timestamp_writes: ts(4),
            });
            pass.set_pipeline(&self.composite_pipeline);
            pass.set_bind_group(0, &composite_bind, &[]);
            pass.dispatch_workgroups(gx, gy, 1);
        }
        {
            // Stage 6 BLEND — composite transparents over the lit result, before TAA.
            // Always recorded for a valid timestamp; dispatched only when the scene has
            // transparent voxels (then TAA reads `color_blended` instead of `color`).
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("blend pass"),
                timestamp_writes: ts(6),
            });
            if let Some(bind) = &blend_bind {
                pass.set_pipeline(&self.blend_pipeline);
                pass.set_bind_group(0, bind, &[]);
                pass.dispatch_workgroups(gx, gy, 1);
            }
        }
        {
            // Stage 5 TAA — reproject + variance-clip + accumulate → screen + history.
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("taa pass"),
                timestamp_writes: ts(5),
            });
            pass.set_pipeline(&self.taa_pipeline);
            pass.set_bind_group(0, &taa_bind, &[]);
            pass.dispatch_workgroups(gx, gy, 1);
        }

        // Capture this frame's depth into prev_depth for next frame's reprojection
        // (after the TAA has read the old prev_depth).
        encoder.copy_texture_to_texture(
            gbuf.depth.as_image_copy(),
            gbuf.prev_depth.as_image_copy(),
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        if let Some(t) = timing {
            let n = u32::try_from(2 * NUM_TIMED_STAGES).expect("stage count fits u32");
            encoder.resolve_query_set(&t.query_set, 0..n, &t.resolve, 0);
            encoder.copy_buffer_to_buffer(&t.resolve, 0, &t.readback, 0, u64::from(n) * 8);
        }

        // Advance temporal state for the next frame.
        self.prev_camera = camera;
        self.frame_index = self.frame_index.wrapping_add(1);
        self.history_valid = true;
    }

    /// Maps the most recent [`render_timed`](Self::render_timed) timestamps and
    /// returns the per-pass GPU durations in nanoseconds — one per timed stage, in
    /// the order of [`RENDER_STAGE_LABELS`]. `None` when the device lacks timestamp
    /// support. Call after the encoder has been submitted and the device polled.
    #[allow(clippy::cast_precision_loss)]
    pub fn last_stage_times_ns(&self) -> Result<Option<[f64; NUM_TIMED_STAGES]>, GpuError> {
        let Some(t) = self.timing.as_ref() else {
            return Ok(None);
        };
        let slice = t.readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|_| GpuError::Poll)?;
        rx.recv().map_err(|_| GpuError::Poll)??;

        let data = slice.get_mapped_range();
        let mut out = [0.0_f64; NUM_TIMED_STAGES];
        for (stage, slot) in out.iter_mut().enumerate() {
            let b = stage * 16;
            let begin = u64::from_le_bytes(data[b..b + 8].try_into().expect("8 bytes"));
            let end = u64::from_le_bytes(data[b + 8..b + 16].try_into().expect("8 bytes"));
            *slot = end.saturating_sub(begin) as f64 * f64::from(t.period);
        }
        drop(data);
        t.readback.unmap();
        Ok(Some(out))
    }

    /// Total GPU compute time of the last timed render (sum of all stages), ns.
    /// `None` without timestamp support.
    pub fn last_kernel_ns(&self) -> Result<Option<f64>, GpuError> {
        Ok(self.last_stage_times_ns()?.map(|s| s.iter().sum()))
    }
}
