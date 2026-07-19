//! The GPU-resident render path: one compute dispatch builds a camera ray per
//! pixel, traverses, shades the hit, and writes color straight to a storage
//! texture — no readback, no CPU ray-gen, no CPU shading. The viewer blits the
//! resulting texture to its surface.

use wgpu::util::DeviceExt;

use voxel_core::{GpuCamera, MaterialTable, NodeLayout, SchoolBBuffer};

use crate::buffers;
use crate::context::GpuContext;
use crate::error::GpuError;

/// The output storage-texture format the render kernel writes.
pub const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Reusable compute-pass timestamp resources (present iff the device supports
/// `TIMESTAMP_QUERY`): a 2-slot query set and the resolve/readback buffers.
/// Mirrors the traverser's timing so the viewer's render kernel can be measured
/// on the GPU timeline (readback of two timestamps only, no per-pixel copy).
struct RenderTiming {
    query_set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    /// Nanoseconds per timestamp tick.
    period: f32,
}

/// The per-scene shading mode, chosen by [`SchoolBBuffer::has_leaf_color`]. The
/// two arms bind different buffers at slots 5..8 and compile a different entry
/// shader; the palette arm is byte-identical to the pre-truecolor renderer.
///
/// The selection is a silent fallback by design: a scene with no baked colour —
/// including an *empty* truecolor bake (zero occupied voxels) — routes to the
/// palette arm rather than erroring.
enum RenderMode {
    /// Palette materials (`docs/materials/02-03`): `leaf_mat`@5 + `material_table`@6.
    Palette {
        /// Per-leaf packed material slots; `COPY_DST` for
        /// [`update_leaf_mat`](GpuRenderer::update_leaf_mat).
        leaf_mat_buf: wgpu::Buffer,
        /// The global `global_id → colour` table; scene-static.
        table_buf: wgpu::Buffer,
    },
    /// Per-voxel truecolor: `leaf_color_page`@5 (per-leaf base offset) + N colour
    /// chunks at 6..8. Two provenances, same bindings and shader:
    /// - **static** (build-once, [`new`]): `page_buf` is the prefix sum of
    ///   `count_occupied`, chunks are `STORAGE`-only. `editable == false`; the
    ///   paged edit methods refuse.
    /// - **paged** (editable, [`new_paged`]): `page_buf` is the editable colour
    ///   pool's per-leaf page offset, chunks are `STORAGE | COPY_DST` and grow on
    ///   demand. `editable == true`.
    ///
    /// [`new`]: GpuRenderer::new
    /// [`new_paged`]: GpuRenderer::new_paged
    Truecolor {
        /// Per-leaf colour base offset into the pool (prefix sum for the static
        /// path, editable page offset for the paged path). `COPY_DST` either way
        /// (a static renderer just never patches it).
        page_buf: wgpu::Buffer,
        /// The physical colour sub-buffers (`chunk i` covers pool entries
        /// `[i·per_chunk, (i+1)·per_chunk)`). The paged path grows/adds these.
        chunks: Vec<wgpu::Buffer>,
        /// One shared 1-`u32` buffer bound into the unused chunk slots `[N, N_MAX)`.
        dummy_buf: wgpu::Buffer,
        /// Pool entries per chunk (== the CPU pool's `chunk_entries` for the paged
        /// path). Drives the chunk-select in edit-method offsets.
        per_chunk: u32,
        /// Whether the paged edit methods ([`update_color_page`] etc.) apply. The
        /// static build-once path is `false`.
        ///
        /// [`update_color_page`]: GpuRenderer::update_color_page
        editable: bool,
    },
}

/// A compiled render pipeline with one uploaded structure.
pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    node_buf: wgpu::Buffer,
    leaf_buf: wgpu::Buffer,
    bounds_buf: wgpu::Buffer,
    /// The shading mode + its slot-5..8 buffers (palette vs truecolor).
    mode: RenderMode,
    camera_buf: wgpu::Buffer,
    /// The hover-cursor uniform (@9). Zero — WebGPU zero-initializes buffers —
    /// means inactive, so a renderer that never calls
    /// [`set_cursor`](Self::set_cursor) renders byte-identically to a
    /// cursor-free build.
    cursor_buf: wgpu::Buffer,
    /// The themed-sky uniform (@10), initialized to the product's original
    /// gradient so a renderer that never calls [`set_sky`](Self::set_sky)
    /// looks as it always did.
    sky_buf: wgpu::Buffer,
    /// Max storage-buffer binding size, kept so [`reupload`](Self::reupload) can
    /// rebuild the structure buffers after a topology edit without the context.
    max_binding: u64,
    timing: Option<RenderTiming>,
}

/// Builds the bind-group layout, entry-shader source, and slot-5..8 buffers for
/// the scene's shading mode (truecolor when `structure.has_leaf_color()`, else
/// palette). Bindings 0..4 are shared and added by the caller's layout list here.
fn select_render_mode(
    device: &wgpu::Device,
    ctx: &GpuContext,
    structure: &SchoolBBuffer,
    table: &MaterialTable,
    per_chunk: u32,
    max_binding: u64,
    output_entry: wgpu::BindGroupLayoutEntry,
) -> Result<(wgpu::BindGroupLayout, String, RenderMode), GpuError> {
    if structure.has_leaf_color() {
        // Truecolor: the WGSL + layout hardcode N_MAX_CHUNKS chunk bindings.
        debug_assert_eq!(
            buffers::N_MAX_CHUNKS,
            3,
            "render_truecolor.wgsl + the layout below bind exactly 3 chunk slots"
        );
        // Probe device limits FIRST — a failure leaves no partial GPU state.
        let n_res = structure.resolution().voxels_per_axis();
        let len = structure.leaf_color_words().len();
        let base_bytes = (structure.leaf_color_base_words().len() * 4) as u64;
        buffers::probe_truecolor(
            n_res,
            len,
            base_bytes,
            per_chunk,
            ctx.max_storage_buffers(),
            max_binding,
            ctx.max_buffer_size(),
        )?;
        let (chunks, base_buf, dummy_buf) = buffers::upload_color_chunks(
            device,
            structure.leaf_color_words(),
            structure.leaf_color_base_words(),
            per_chunk,
        );
        let layout = truecolor_layout(device, output_entry);
        // Scene-time pipeline selection: a scene with transparent leaves compiles the
        // front-to-back BLEND entry; otherwise the byte-identical opaque entry, so the
        // opaque path never pays for compositing. Same 7-binding layout either way.
        let source = if structure.has_transparency() {
            buffers::color_blend_shader_source(per_chunk, buffers::MAX_BLEND)
        } else {
            buffers::color_shader_source(per_chunk)
        };
        Ok((
            layout,
            source,
            RenderMode::Truecolor {
                page_buf: base_buf,
                chunks,
                dummy_buf,
                per_chunk,
                editable: false,
            },
        ))
    } else {
        let (leaf_mat_buf, table_buf) =
            buffers::upload_materials(device, structure, table, max_binding)?;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render layout"),
            entries: &[
                buffers::storage_entry(0, true), // nodes
                buffers::storage_entry(1, true), // leaf_words
                buffers::storage_entry(2, true), // leaf_bounds
                buffers::uniform_entry(3),       // camera
                output_entry,                    // output @4
                buffers::storage_entry(5, true), // leaf_mat
                buffers::storage_entry(6, true), // material_table
                buffers::uniform_entry(9),       // hover cursor (zero = inactive)
                buffers::uniform_entry(10),      // themed sky gradient
            ],
        });
        Ok((
            layout,
            buffers::render_shader_source(include_str!("../shaders/render.wgsl")),
            RenderMode::Palette {
                leaf_mat_buf,
                table_buf,
            },
        ))
    }
}

/// The truecolor bind-group layout (9 entries: 3 structure + camera + output +
/// `leaf_color_page` + 3 colour chunks). Identical for the static and paged
/// paths, so both build it here.
fn truecolor_layout(
    device: &wgpu::Device,
    output_entry: wgpu::BindGroupLayoutEntry,
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("render layout (truecolor)"),
        entries: &[
            buffers::storage_entry(0, true), // nodes
            buffers::storage_entry(1, true), // leaf_words
            buffers::storage_entry(2, true), // leaf_bounds
            buffers::uniform_entry(3),       // camera
            output_entry,                    // output @4
            buffers::storage_entry(5, true), // leaf_color_page
            buffers::storage_entry(6, true), // leaf_color_0
            buffers::storage_entry(7, true), // leaf_color_1
            buffers::storage_entry(8, true), // leaf_color_2
            buffers::uniform_entry(9),       // hover cursor (zero = inactive)
            buffers::uniform_entry(10),      // themed sky gradient
        ],
    })
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

/// Builds the layout, entry-shader source, and slot-5..8 buffers for the
/// **paged** (editable) truecolor path (brush-editing Stage A2). Mirrors
/// [`select_render_mode`]'s truecolor arm but sources its buffers from the tree's
/// colour pool (`pages`) with `COPY_DST` usages, and reads `per_chunk` from the
/// pool so CPU page placement and GPU chunk-select stay aligned.
fn select_paged_mode(
    device: &wgpu::Device,
    ctx: &GpuContext,
    structure: &SchoolBBuffer,
    pages: &voxel_core::ColorPages,
    per_chunk: u32,
    max_binding: u64,
    output_entry: wgpu::BindGroupLayoutEntry,
) -> Result<(wgpu::BindGroupLayout, String, RenderMode), GpuError> {
    debug_assert_eq!(
        buffers::N_MAX_CHUNKS,
        3,
        "the truecolor shader + layout bind exactly 3 chunk slots"
    );
    let n_res = structure.resolution().voxels_per_axis();
    let pool_image = build_pool_image(pages);
    let page_words = structure.leaf_color_page_words();
    let base_bytes = (page_words.len() * 4) as u64;
    buffers::probe_truecolor(
        n_res,
        pool_image.len(),
        base_bytes,
        per_chunk,
        ctx.max_storage_buffers(),
        max_binding,
        ctx.max_buffer_size(),
    )?;
    let (chunks, page_buf, dummy_buf) =
        buffers::upload_color_pool(device, &pool_image, page_words, per_chunk);
    let layout = truecolor_layout(device, output_entry);
    // Same scene-time opaque/BLEND selection as the static path, but keyed on the
    // tree colour store's transparency (from_sparse derived the bounds bits from it).
    let source = if pages.has_transparency() {
        buffers::color_blend_shader_source(per_chunk, buffers::MAX_BLEND)
    } else {
        buffers::color_shader_source(per_chunk)
    };
    Ok((
        layout,
        source,
        RenderMode::Truecolor {
            page_buf,
            chunks,
            dummy_buf,
            per_chunk,
            editable: true,
        },
    ))
}

impl GpuRenderer {
    /// Compiles the render kernel and uploads `structure` plus its material data
    /// (`table` is the global `global_id → colour` table; pass
    /// [`MaterialTable::missing_only`] when rendering occupancy-only fixtures —
    /// the shader falls back to position shading for global-0).
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
        // bindings 0..4 (nodes/leaf_words/leaf_bounds/camera/output) are shared by
        // both modes; slots 5..8 and the entry shader differ.
        let output_entry = buffers::storage_texture_entry(4, OUTPUT_FORMAT);
        let (layout, shader_src, mode) = select_render_mode(
            &device,
            ctx,
            structure,
            table,
            per_chunk,
            max_binding,
            output_entry,
        )?;
        Ok(Self::assemble(
            ctx,
            device,
            queue,
            node_buf,
            leaf_buf,
            bounds_buf,
            layout,
            shader_src,
            mode,
            max_binding,
        ))
    }

    /// Builds an **editable paged** truecolor renderer (brush-editing Stage A2)
    /// from a tree's colour pool ([`SparseTree::color_pages`]). `structure` is the
    /// [`SchoolBBuffer::from_sparse`] of the same tree (it supplies the derived
    /// `leaf_color_page` table and the transparency bits). The hit-read is
    /// byte-identical to [`new`](Self::new)'s static path — only the page offsets
    /// (editable pool pages vs prefix sums) and the buffer usages (`COPY_DST`,
    /// growable) differ — so a paged scene renders identically to its build-once
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
        let output_entry = buffers::storage_texture_entry(4, OUTPUT_FORMAT);
        let (layout, shader_src, mode) = select_paged_mode(
            &device,
            ctx,
            structure,
            pages,
            per_chunk,
            max_binding,
            output_entry,
        )?;
        Ok(Self::assemble(
            ctx,
            device,
            queue,
            node_buf,
            leaf_buf,
            bounds_buf,
            layout,
            shader_src,
            mode,
            max_binding,
        ))
    }

    /// The shared construction tail: given the uploaded structure buffers, the
    /// bind-group layout, the entry-shader source, and the slot-5..8 `mode`,
    /// creates the camera buffer, compiles the pipeline, and wires up optional
    /// GPU-timestamp resources. Used by both [`new_with_per_chunk`](Self::new_with_per_chunk)
    /// and [`new_paged`](Self::new_paged).
    #[allow(clippy::too_many_arguments)]
    fn assemble(
        ctx: &GpuContext,
        device: wgpu::Device,
        queue: wgpu::Queue,
        node_buf: wgpu::Buffer,
        leaf_buf: wgpu::Buffer,
        bounds_buf: wgpu::Buffer,
        layout: wgpu::BindGroupLayout,
        shader_src: String,
        mode: RenderMode,
        max_binding: u64,
    ) -> Self {
        let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: std::mem::size_of::<GpuCamera>() as u64,
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
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render"),
            source: wgpu::ShaderSource::Wgsl(shader_src.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("render pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("render pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("render_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let timing = ctx.supports_timestamps().then(|| {
            let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("render timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            });
            let resolve = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("render ts resolve"),
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("render ts readback"),
                size: 16,
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
            pipeline,
            layout,
            node_buf,
            leaf_buf,
            bounds_buf,
            mode,
            camera_buf,
            cursor_buf,
            sky_buf,
            max_binding,
            timing,
        }
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
    /// build. One 32-byte uniform write; cost only exists on hits.
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

    /// The paged (editable) truecolor `per_chunk` if this is such a renderer.
    fn paged_per_chunk(&self) -> Option<u32> {
        match self.mode {
            RenderMode::Truecolor {
                editable: true,
                per_chunk,
                ..
            } => Some(per_chunk),
            _ => None,
        }
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
        if let RenderMode::Truecolor { chunks, .. } = &self.mode {
            self.queue
                .write_buffer(&chunks[chunk], local * 4, bytemuck::cast_slice(words));
        }
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
        let RenderMode::Truecolor {
            editable: true,
            page_buf,
            ..
        } = &self.mode
        else {
            return Err(GpuError::Unsupported {
                n: 0,
                reason: "update_page_word requires a paged (editable) truecolor renderer",
            });
        };
        self.queue.write_buffer(
            page_buf,
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
        let RenderMode::Truecolor { chunks, .. } = &mut self.mode else {
            unreachable!("paged_per_chunk confirmed truecolor");
        };
        while chunks.len() <= chunk {
            chunks.push(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("leaf_color_pool_grow"),
                size: full_bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }));
        }
        if chunks[chunk].size() < want_bytes {
            let grown = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("leaf_color_pool_grow"),
                size: full_bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let old_size = chunks[chunk].size();
            let mut enc = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("grow chunk copy"),
                });
            enc.copy_buffer_to_buffer(&chunks[chunk], 0, &grown, 0, old_size);
            self.queue.submit(std::iter::once(enc.finish()));
            chunks[chunk] = grown;
        }
        Ok(())
    }

    /// Replaces the resident structure and page table after a topology edit on a
    /// paged truecolor scene — the paged analogue of [`reupload`](Self::reupload).
    /// Rebuilds nodes/leaves/bounds and the `leaf_color_page` buffer from
    /// `structure` (a fresh [`SchoolBBuffer::from_sparse`]); the colour **pool is
    /// untouched**, so the ~hundreds-of-MB colour data is never re-uploaded. The
    /// caller re-uploads only the pages the edit actually changed via
    /// [`update_color_page`](Self::update_color_page).
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
        let new_page = wgpu::util::DeviceExt::create_buffer_init(
            &self.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("leaf_color_page"),
                contents: bytemuck::cast_slice(&page_words),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            },
        );
        if let RenderMode::Truecolor { page_buf, .. } = &mut self.mode {
            *page_buf = new_page;
        }
        Ok(())
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
    ///
    /// Works for the palette path and the **paged** (editable) truecolor path —
    /// both keep `leaf_buf`/`bounds_buf` `COPY_DST` — but the caller must also sync
    /// the touched leaf's colour page on a paged scene (via
    /// [`update_color_page`](Self::update_color_page)).
    ///
    /// # Errors
    /// Returns [`GpuError::Unsupported`] on a **static** truecolor renderer:
    /// per-voxel colour is build-once there, so the scene must be re-baked via
    /// [`new`](Self::new).
    pub fn update_leaf(&self, structure: &SchoolBBuffer, leaf_idx: u32) -> Result<(), GpuError> {
        if matches!(
            self.mode,
            RenderMode::Truecolor {
                editable: false,
                ..
            }
        ) {
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
    /// material edit ([`Edit::Material { spilled: false, .. }`]). `structure` must
    /// already have had [`SchoolBBuffer::patch_leaf_mat`] applied for `leaf_idx`;
    /// this copies that leaf's `STRIDE_W` words (at `leaf_idx * STRIDE_W * 4`
    /// bytes) via one `queue.write_buffer` — the same O(1) shape as
    /// [`update_leaf`](Self::update_leaf). A *spilled* edit bumps the topology
    /// generation and must go through [`reupload`](Self::reupload) instead.
    ///
    /// [`Edit::Material { spilled: false, .. }`]: voxel_core::Edit::Material
    ///
    /// # Errors
    /// Returns [`GpuError::Unsupported`] on a truecolor renderer (no `leaf_mat`
    /// buffer exists; colour is build-once — re-bake via [`new`](Self::new)).
    pub fn update_leaf_mat(
        &self,
        structure: &SchoolBBuffer,
        leaf_idx: u32,
    ) -> Result<(), GpuError> {
        let RenderMode::Palette { leaf_mat_buf, .. } = &self.mode else {
            return Err(GpuError::Unsupported {
                n: structure.resolution().voxels_per_axis(),
                reason: "truecolor renderer is build-once; re-bake via GpuRenderer::new after an edit",
            });
        };
        let stride_w = voxel_core::palette::STRIDE_W;
        let base = leaf_idx as usize * stride_w;
        let slot = &structure.leaf_mat_words()[base..base + stride_w];
        self.queue
            .write_buffer(leaf_mat_buf, (base * 4) as u64, bytemuck::cast_slice(slot));
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
    ///
    /// # Errors
    /// Returns [`GpuError::BufferTooLarge`] if a structure buffer exceeds the
    /// binding cap, or [`GpuError::Unsupported`] on a truecolor renderer (per-voxel
    /// colour is build-once — a topology edit renumbers leaves and invalidates the
    /// colour chunks; re-bake via [`new`](Self::new)).
    pub fn reupload(&mut self, structure: &SchoolBBuffer) -> Result<(), GpuError> {
        if !matches!(self.mode, RenderMode::Palette { .. }) {
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
        // edit (renumbered leaves, possibly a spilled material change) invalidates
        // them too — rebuild `leaf_mat`. The global colour `table` is unchanged by
        // a topology/material edit, so its buffer stays.
        let stride_w = voxel_core::palette::STRIDE_W;
        let mut mat_words = structure.leaf_mat_words().to_vec();
        if mat_words.is_empty() {
            mat_words = vec![0u32; stride_w];
        }
        let new_mat = wgpu::util::DeviceExt::create_buffer_init(
            &self.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("leaf_mat"),
                contents: bytemuck::cast_slice(&mat_words),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            },
        );
        // Guard above guarantees the Palette arm; assign the rebuilt buffer.
        if let RenderMode::Palette { leaf_mat_buf, .. } = &mut self.mode {
            *leaf_mat_buf = new_mat;
        }
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
        &self,
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
        &self,
        encoder: &mut wgpu::CommandEncoder,
        camera: &GpuCamera,
        output: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        self.record(encoder, camera, output, width, height, true);
    }

    /// Shared pass recorder; when `timed` and timestamps are available, writes
    /// the begin/end timestamps and resolves them into the readback buffer.
    fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        camera: &GpuCamera,
        output: &wgpu::TextureView,
        width: u32,
        height: u32,
        timed: bool,
    ) {
        self.queue
            .write_buffer(&self.camera_buf, 0, bytemuck::bytes_of(camera));

        let mut entries = vec![
            buffers::bind(0, self.node_buf.as_entire_binding()),
            buffers::bind(1, self.leaf_buf.as_entire_binding()),
            buffers::bind(2, self.bounds_buf.as_entire_binding()),
            buffers::bind(3, self.camera_buf.as_entire_binding()),
            buffers::bind(4, wgpu::BindingResource::TextureView(output)),
            buffers::bind(9, self.cursor_buf.as_entire_binding()),
            buffers::bind(10, self.sky_buf.as_entire_binding()),
        ];
        match &self.mode {
            RenderMode::Palette {
                leaf_mat_buf,
                table_buf,
            } => {
                entries.push(buffers::bind(5, leaf_mat_buf.as_entire_binding()));
                entries.push(buffers::bind(6, table_buf.as_entire_binding()));
            }
            RenderMode::Truecolor {
                page_buf,
                chunks,
                dummy_buf,
                ..
            } => {
                entries.push(buffers::bind(5, page_buf.as_entire_binding()));
                // Slots 6..8: real chunk `i` if present, else the shared dummy. The
                // probe guaranteed `N <= N_MAX` and every valid hit `g < N*PER_CHUNK`,
                // so a dummy slot is bound but never indexed by a real read.
                for i in 0..buffers::N_MAX_CHUNKS {
                    let buf = chunks.get(i as usize).unwrap_or(dummy_buf);
                    entries.push(buffers::bind(6 + i, buf.as_entire_binding()));
                }
            }
        }
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("render bind group"),
            layout: &self.layout,
            entries: &entries,
        });

        let timing = if timed { self.timing.as_ref() } else { None };
        let timestamp_writes = timing.map(|t| wgpu::ComputePassTimestampWrites {
            query_set: &t.query_set,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("render pass"),
                timestamp_writes,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }

        if let Some(t) = timing {
            encoder.resolve_query_set(&t.query_set, 0..2, &t.resolve, 0);
            encoder.copy_buffer_to_buffer(&t.resolve, 0, &t.readback, 0, 16);
        }
    }

    /// Maps the most recent [`render_timed`](Self::render_timed) timestamp pair
    /// and returns the compute-pass duration in nanoseconds, or `None` when the
    /// device lacks timestamp support. Call after the encoder has been submitted
    /// and the device polled to completion.
    #[allow(clippy::cast_precision_loss)]
    pub fn last_kernel_ns(&self) -> Result<Option<f64>, GpuError> {
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
        let begin = u64::from_le_bytes(data[0..8].try_into().expect("8 bytes"));
        let end = u64::from_le_bytes(data[8..16].try_into().expect("8 bytes"));
        drop(data);
        t.readback.unmap();
        Ok(Some(end.saturating_sub(begin) as f64 * f64::from(t.period)))
    }
}
