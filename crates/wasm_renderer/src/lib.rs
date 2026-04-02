//! GPU-Resident Voxel Renderer — WASM entry point for the WebGPU worker pipeline.

pub mod camera;
pub mod mesh_cpu;
pub mod obj_parser;
pub mod pool;
pub mod scene;
pub mod summary_cpu;
pub mod voxelizer_cpu;

// GPU module and Renderer struct are WASM-only (depend on web_sys, OffscreenCanvas).
// Camera and pool are pure Rust, testable on any target.
#[cfg(target_arch = "wasm32")]
pub mod gpu;
#[cfg(target_arch = "wasm32")]
pub mod passes;
#[cfg(target_arch = "wasm32")]
pub mod pool_gpu;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    depth_texture: Option<wgpu::Texture>,
    depth_view: Option<wgpu::TextureView>,
    hiz_texture: Option<wgpu::Texture>,
    hiz_mip_views: Vec<wgpu::TextureView>,
    hiz_mip_count: u32,
    hiz_build_pass: passes::hiz_build::HizBuildPass,
    hiz_bind_groups: Option<passes::hiz_build::HizBindGroups>,
    occlusion_cull_pass: passes::occlusion_cull::OcclusionCullPass,
    render: gpu::RenderResources,
    camera: camera::Camera,
    pool: pool_gpu::ChunkPool,
    summary_pass: passes::summary::SummaryPass,
    mesh_count_pass: passes::mesh_count::MeshCountPass,
    mesh_pass: passes::mesh_rebuild::MeshPass,
    prefix_sum_pass: passes::prefix_sum::PrefixSumPass,
    build_indirect_pass: passes::build_indirect::BuildIndirectPass,
    // F8: Lazy — created when wireframe mode is first activated.
    build_wireframe_pass: Option<passes::build_wireframe::BuildWireframePass>,
    resident_count: u32,
    total_voxels: u32,
    mesh_verts: u32,
    mesh_indices: u32,
    mesh_quads: u32,
    render_mode: u8,
    backface_culling: bool,
    depth_prepass_enabled: bool,
    use_cpu_mesh: bool,
    frame_index: u32,
    // Per-frame CPU-side pass timing (real, not fabricated)
    timing_depth_ms: f32,
    timing_color_ms: f32,
    timing_total_ms: f32,
    // Per-frame diagnostic counters
    diag_summary_rebuilds: u32,
    diag_mesh_rebuilds: u32,
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl Renderer {
    #[wasm_bindgen(constructor)]
    pub async fn new(
        canvas: web_sys::HtmlCanvasElement,
        width: u32,
        height: u32,
    ) -> Result<Renderer, JsValue> {
        // Instance — no display handle needed for WASM/WebGPU
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::BROWSER_WEBGPU;
        let instance = wgpu::Instance::new(desc);

        // Surface from HTMLCanvasElement (main thread — ADR-0014)
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .map_err(|e| JsValue::from_str(&format!("Surface error: {e}")))?;

        // Adapter — returns Result in v29
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|e| JsValue::from_str(&format!("Adapter error: {e}")))?;

        // Device + Queue
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("gestalt-renderer"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits {
                    max_buffer_size: 512 * 1024 * 1024,
                    max_storage_buffer_binding_size: 512 * 1024 * 1024,
                    ..wgpu::Limits::downlevel_webgl2_defaults()
                },
                experimental_features: wgpu::ExperimentalFeatures::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| JsValue::from_str(&format!("Device error: {e}")))?;

        // Surface config
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        // Chunk pool — all GPU buffers for 1024 slots
        let pool = pool_gpu::ChunkPool::new(&device);

        // Compute passes
        let summary_pass = passes::summary::SummaryPass::new(
            &device,
            pool.summary_compute_layout(),
        );
        let mesh_count_pass = passes::mesh_count::MeshCountPass::new(
            &device,
            pool.mesh_count_layout(),
        );
        let mesh_pass = passes::mesh_rebuild::MeshPass::new(
            &device,
            pool.mesh_compute_layout(),
        );
        let prefix_sum_pass = passes::prefix_sum::PrefixSumPass::new(
            &device,
            pool.prefix_sum_layout(),
        );
        let build_indirect_pass = passes::build_indirect::BuildIndirectPass::new(
            &device,
            pool.mesh_offset_table_buf(),
            pool.indirect_buffer(),
            pool.visibility_buf(),
        );
        // F8: BuildWireframePass created lazily when wireframe mode first activated.
        // Avoids allocating ~128 MB wireframe buffers at startup.

        // Depth + Hi-Z + render resources
        let depth_format = wgpu::TextureFormat::Depth32Float;
        let (depth_tex, depth_view) = gpu::create_depth_texture(&device, width, height, depth_format);
        let (hiz_tex, hiz_mip_views, hiz_mip_count) = gpu::create_hiz_pyramid(&device, width, height);
        let hiz_build_pass = passes::hiz_build::HizBuildPass::new(&device);
        let hiz_bind_groups = hiz_build_pass.create_bind_groups(
            &device, &depth_view, &hiz_mip_views, width, height,
        );
        let occlusion_cull_pass = passes::occlusion_cull::OcclusionCullPass::new(&device);
        let render = gpu::RenderResources::new(
            &device,
            format,
            depth_format,
            pool.vertex_pool_buf(),
            pool.scene_global_layout(),
            pool.scene_global_bind_group(),
        );
        let camera = camera::Camera::new(width as f32, height as f32);

        log("Renderer initialized");

        Ok(Renderer {
            device,
            queue,
            surface,
            surface_config: config,
            depth_texture: Some(depth_tex),
            depth_view: Some(depth_view),
            hiz_texture: Some(hiz_tex),
            hiz_mip_views,
            hiz_mip_count,
            hiz_build_pass,
            hiz_bind_groups: Some(hiz_bind_groups),
            occlusion_cull_pass,
            render,
            camera,
            pool,
            summary_pass,
            mesh_count_pass,
            mesh_pass,
            prefix_sum_pass,
            build_indirect_pass,
            build_wireframe_pass: None,
            resident_count: 0,
            total_voxels: 0,
            mesh_verts: 0,
            mesh_indices: 0,
            mesh_quads: 0,
            render_mode: 0x00,
            backface_culling: true,
            depth_prepass_enabled: true,
            use_cpu_mesh: false,
            frame_index: 0,
            timing_depth_ms: 0.0,
            timing_color_ms: 0.0,
            timing_total_ms: 0.0,
            diag_summary_rebuilds: 0,
            diag_mesh_rebuilds: 0,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);

        let (depth_tex, depth_view) = gpu::create_depth_texture(
            &self.device, width, height, wgpu::TextureFormat::Depth32Float,
        );
        let (hiz_tex, hiz_mip_views, hiz_mip_count) =
            gpu::create_hiz_pyramid(&self.device, width, height);
        let hiz_bind_groups = self.hiz_build_pass.create_bind_groups(
            &self.device, &depth_view, &hiz_mip_views, width, height,
        );

        self.depth_texture = Some(depth_tex);
        self.depth_view = Some(depth_view);
        self.hiz_texture = Some(hiz_tex);
        self.hiz_mip_views = hiz_mip_views;
        self.hiz_mip_count = hiz_mip_count;
        self.hiz_bind_groups = Some(hiz_bind_groups);

        self.camera.resize(width as f32, height as f32);
    }

    pub fn set_render_mode(&mut self, mode: u8) {
        self.render_mode = mode;
    }

    // ── Stats getters (cheap CPU-side reads) ──

    pub fn get_resident_count(&self) -> u32 { self.resident_count }
    pub fn get_render_mode(&self) -> u8 { self.render_mode }
    pub fn get_frame_index(&self) -> u32 { self.frame_index }
    pub fn get_total_voxels(&self) -> u32 { self.total_voxels }
    pub fn get_mesh_verts(&self) -> u32 { self.mesh_verts }
    pub fn get_mesh_indices(&self) -> u32 { self.mesh_indices }
    pub fn get_mesh_quads(&self) -> u32 { self.mesh_quads }

    pub fn get_camera_pos(&self) -> Vec<f32> {
        let p = self.camera.position();
        vec![p.x, p.y, p.z]
    }

    pub fn get_camera_dir(&self) -> Vec<f32> {
        let d = self.camera.direction();
        vec![d.x, d.y, d.z]
    }

    pub fn get_camera_fov(&self) -> f32 { self.camera.fov_y() }
    pub fn get_camera_near(&self) -> f32 { self.camera.near() }
    pub fn get_camera_far(&self) -> f32 { self.camera.far() }
    pub fn get_camera_aspect(&self) -> f32 { self.camera.aspect() }
    pub fn get_viewport_width(&self) -> u32 { self.surface_config.width }
    pub fn get_viewport_height(&self) -> u32 { self.surface_config.height }
    pub fn get_free_slot_count(&self) -> u32 { self.pool.allocator().free_count() }
    pub fn get_has_wireframe(&self) -> bool { self.build_wireframe_pass.is_some() }
    pub fn get_backface_culling(&self) -> bool { self.backface_culling }
    pub fn get_depth_prepass_enabled(&self) -> bool { self.depth_prepass_enabled }

    pub fn set_fov(&mut self, degrees: f32) { self.camera.set_fov(degrees); }
    pub fn set_backface_culling(&mut self, enabled: bool) { self.backface_culling = enabled; }
    pub fn set_depth_prepass(&mut self, enabled: bool) { self.depth_prepass_enabled = enabled; }
    pub fn set_use_cpu_mesh(&mut self, enabled: bool) { self.use_cpu_mesh = enabled; }
    pub fn get_use_cpu_mesh(&self) -> bool { self.use_cpu_mesh }

    /// Per-pass CPU-side frame timing: [depth_ms, color_ms, total_ms].
    /// Measures command encoding time, not GPU execution. Real data, not fabricated.
    pub fn get_pass_timings(&self) -> Vec<f32> {
        vec![self.timing_depth_ms, self.timing_color_ms, self.timing_total_ms]
    }

    /// Diagnostic counters: [summary_rebuilds, mesh_rebuilds].
    /// Currently 0 (compute runs at load time, not per-frame). Phase 4 will populate.
    pub fn get_diag_counters(&self) -> Vec<u32> {
        vec![self.diag_summary_rebuilds, self.diag_mesh_rebuilds]
    }

    pub fn set_camera(&mut self, px: f32, py: f32, pz: f32, dx: f32, dy: f32, dz: f32) {
        self.camera.set_look(
            glam::Vec3::new(px, py, pz),
            glam::Vec3::new(dx, dy, dz),
        );
    }

    /// Generate and upload the procedural test scene (room + sphere + emissive).
    pub fn load_test_scene(&mut self) -> Result<(), JsValue> {
        let (chunks, materials) = scene::generate_test_scene();

        // Upload material table
        let mat_bytes: &[u8] = bytemuck::cast_slice(&materials);
        self.pool.upload_materials(&self.queue, mat_bytes);

        // Upload each chunk
        for chunk in &chunks {
            let slot = self.pool.alloc_slot(chunk.coord)
                .map_err(|e| JsValue::from_str(&format!("Alloc error: {e:?}")))?;
            let palette_words = chunk.palette.as_words();
            let bpe = scene::IndexBufBuilder::bits_per_entry(chunk.palette.len());
            let index_buf_words = chunk.index_buf.pack(bpe);
            let meta = scene::IndexBufBuilder::palette_meta(chunk.palette.len());
            self.pool.upload_chunk(
                &self.queue,
                slot,
                chunk.coord,
                chunk.occupancy.as_words(),
                &palette_words,
                &index_buf_words,
                meta,
            );
            log(&format!(
                "Uploaded chunk ({},{},{}) → slot {}, {} voxels, palette {} entries (bpe={})",
                chunk.coord.x, chunk.coord.y, chunk.coord.z,
                slot, chunk.occupancy.popcount(),
                chunk.palette.len(), bpe,
            ));
        }

        // Dispatch I-3 summary rebuild for all uploaded chunks
        let resident_count = self.pool.allocator().resident_count();
        {
            let mut encoder = self.device.create_command_encoder(
                &wgpu::CommandEncoderDescriptor { label: Some("i3-summary") },
            );
            self.summary_pass.dispatch(
                &mut encoder,
                self.pool.summary_compute_bind_group(),
                resident_count,
            );
            self.queue.submit(std::iter::once(encoder.finish()));
        }

        self.resident_count = resident_count;
        self.rebuild_meshes(&chunks);

        // CPU reference stats
        self.total_voxels = 0;
        self.mesh_verts = 0;
        self.mesh_indices = 0;
        self.mesh_quads = 0;
        for chunk in &chunks {
            self.total_voxels += chunk.occupancy.popcount();
            let pal_words = chunk.palette.as_words();
            let bpe = scene::IndexBufBuilder::bits_per_entry(chunk.palette.len());
            let idx_words = chunk.index_buf.pack(bpe);
            let meta = scene::IndexBufBuilder::palette_meta(chunk.palette.len());
            let cpu_result = mesh_cpu::mesh_rebuild_cpu(
                chunk.occupancy.as_words(),
                &pal_words,
                &idx_words,
                meta,
                [chunk.coord.x, chunk.coord.y, chunk.coord.z],
            );
            self.mesh_verts += cpu_result.draw_meta.vertex_count;
            self.mesh_indices += cpu_result.draw_meta.index_count;
            self.mesh_quads += cpu_result.quad_count;
            log(&format!(
                "CPU reference: {} verts, {} indices, {} quads",
                cpu_result.draw_meta.vertex_count,
                cpu_result.draw_meta.index_count,
                cpu_result.quad_count,
            ));
        }

        log(&format!(
            "Test scene loaded: {} chunks, {} materials, I-3 + R-1 dispatched for {} slots",
            chunks.len(),
            materials.len(),
            resident_count,
        ));
        Ok(())
    }

    /// Load an OBJ model: parse → voxelize → upload → dispatch I-3 + R-1.
    /// Clears the current scene first.
    pub fn load_obj_model(&mut self, obj_text: &str, resolution: u32) -> Result<(), JsValue> {
        let parsed = obj_parser::parse_obj(obj_text);
        log(&format!(
            "Parsed OBJ: {} vertices, {} triangles, {} materials",
            parsed.positions.len(),
            parsed.triangles.len(),
            parsed.material_names.len(),
        ));

        if parsed.triangles.is_empty() {
            return Err(JsValue::from_str("OBJ contains no triangles"));
        }

        let result = voxelizer_cpu::voxelize(&parsed, resolution);
        log(&format!(
            "Voxelized: {} chunks",
            result.chunks.len(),
        ));

        if result.chunks.is_empty() {
            return Err(JsValue::from_str("Voxelization produced no chunks"));
        }

        // Cap at pool capacity
        let chunk_limit = pool::MAX_SLOTS as usize;
        let chunks_to_load = if result.chunks.len() > chunk_limit {
            log(&format!(
                "WARNING: {} chunks exceeds MAX_SLOTS ({}), loading first {} only",
                result.chunks.len(), chunk_limit, chunk_limit,
            ));
            &result.chunks[..chunk_limit]
        } else {
            &result.chunks[..]
        };

        // Clear existing scene
        self.pool.allocator_mut().clear();

        // Upload material table
        let mat_bytes: &[u8] = bytemuck::cast_slice(&result.materials);
        self.pool.upload_materials(&self.queue, mat_bytes);

        // Upload each chunk
        for chunk in chunks_to_load {
            let slot = self.pool.alloc_slot(chunk.coord)
                .map_err(|e| JsValue::from_str(&format!("Alloc error: {e:?}")))?;
            let palette_words = chunk.palette.as_words();
            let bpe = scene::IndexBufBuilder::bits_per_entry(chunk.palette.len());
            let index_buf_words = chunk.index_buf.pack(bpe);
            let meta = scene::IndexBufBuilder::palette_meta(chunk.palette.len());
            self.pool.upload_chunk(
                &self.queue,
                slot,
                chunk.coord,
                chunk.occupancy.as_words(),
                &palette_words,
                &index_buf_words,
                meta,
            );
        }

        // Dispatch I-3 summary rebuild
        let resident_count = self.pool.allocator().resident_count();
        {
            let mut encoder = self.device.create_command_encoder(
                &wgpu::CommandEncoderDescriptor { label: Some("obj-i3-summary") },
            );
            self.summary_pass.dispatch(
                &mut encoder,
                self.pool.summary_compute_bind_group(),
                resident_count,
            );
            self.queue.submit(std::iter::once(encoder.finish()));
        }

        self.resident_count = resident_count;
        self.rebuild_meshes(chunks_to_load);

        // Update stats
        self.total_voxels = chunks_to_load.iter()
            .map(|c| c.occupancy.popcount()).sum();
        self.mesh_verts = 0;
        self.mesh_indices = 0;
        self.mesh_quads = 0;

        log(&format!(
            "OBJ loaded: {} chunks, {} voxels, {} slots",
            chunks_to_load.len(), self.total_voxels, resident_count,
        ));
        Ok(())
    }

    pub fn render_frame(&mut self) -> Result<(), JsValue> {
        let surface_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(tex)
            | wgpu::CurrentSurfaceTexture::Suboptimal(tex) => tex,
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
            wgpu::CurrentSurfaceTexture::Outdated
            | wgpu::CurrentSurfaceTexture::Lost
            | wgpu::CurrentSurfaceTexture::Validation => {
                self.surface.configure(&self.device, &self.surface_config);
                return Ok(());
            }
        };

        // F8: Lazy wireframe init must happen before any immutable borrows of self.
        if self.render_mode == 0x02 {
            self.ensure_wireframe();
        }

        let color_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let depth_view = self.depth_view.as_ref()
            .ok_or_else(|| JsValue::from_str("No depth view"))?;

        let vp = self.camera.view_proj();
        let pos = self.camera.position();
        self.render.update_camera(&self.queue, &vp, &pos);

        let frame_t0 = js_sys::Date::now();

        let mut encoder = self.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor { label: Some("frame") },
        );

        // ── R-2: Depth prepass ──
        // Skip when: wireframe mode (no solid fill), or user toggled it off for debug.
        let skip_depth = self.render_mode == 0x02 || !self.depth_prepass_enabled;

        // Reset visibility to all-visible and rebuild indirect buffer BEFORE the
        // depth prepass. The depth prepass must draw ALL slots to produce correct
        // depth for Hi-Z. R-4 occlusion cull will then write real visibility,
        // and build_indirect runs again with filtered results for the color pass.
        if !skip_depth && self.resident_count > 0 {
            self.pool.init_visibility(&self.queue, self.resident_count);
            self.build_indirect_pass.dispatch(&mut encoder, self.resident_count);
        }

        if !skip_depth {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("R-2-depth-prepass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            pass.set_pipeline(&self.render.depth_pipeline);
            pass.set_bind_group(0, Some(&self.render.camera_bind_group), &[]);
            pass.set_bind_group(1, Some(&self.render.vertex_bind_group), &[]);
            pass.set_index_buffer(
                self.pool.index_buffer().slice(..),
                wgpu::IndexFormat::Uint32,
            );

            self.draw_all_slots(&mut pass);
        }

        // ── R-3: Hi-Z pyramid build ──
        if !skip_depth {
            if let Some(ref hiz_bgs) = self.hiz_bind_groups {
                self.hiz_build_pass.dispatch(&mut encoder, hiz_bgs);
            }
        }

        // ── R-4: Occlusion cull (chunk-level) + rebuild indirect ──
        if !skip_depth && self.resident_count > 0 {
            if let Some(ref hiz_tex) = self.hiz_texture {
                let hiz_full_view = hiz_tex.create_view(&wgpu::TextureViewDescriptor::default());
                let cull_bg = self.occlusion_cull_pass.create_bind_group(
                    &self.device,
                    &self.render.camera_buf,
                    self.pool.aabb_buf(),
                    self.pool.flags_buf(),
                    &hiz_full_view,
                    self.pool.visibility_buf(),
                    self.resident_count,
                    self.surface_config.width,
                    self.surface_config.height,
                );
                self.occlusion_cull_pass.dispatch(
                    &mut encoder,
                    &cull_bg,
                    self.resident_count,
                );
            }

            // Rebuild indirect draw args with visibility filtering
            self.build_indirect_pass.dispatch(&mut encoder, self.resident_count);
        }

        let after_depth = js_sys::Date::now();

        // ── R-5/R-9: Color pass (mode-dependent) ──

        if self.render_mode == 0x02 {
            // Wireframe: own pass with depth write + clear
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("R-9-wireframe"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.06, g: 0.06, b: 0.08, a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.render.wireframe_pipeline);
            pass.set_bind_group(0, Some(&self.render.camera_bind_group), &[]);
            pass.set_bind_group(1, Some(&self.render.vertex_bind_group), &[]);
            pass.set_index_buffer(
                self.pool.wire_index_pool().slice(..),
                wgpu::IndexFormat::Uint32,
            );
            self.draw_all_slots_wire(&mut pass);
        } else if self.render_mode == 0x10 {
            // Depth viz: fullscreen pass reading depth texture (no depth attachment)
            let depth_viz_bg = self.render.create_depth_viz_bind_group(
                &self.device, depth_view,
            );
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("R-9-depth-viz"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0, g: 0.0, b: 0.0, a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.render.depth_viz_pipeline);
            pass.set_bind_group(0, Some(&self.render.camera_bind_group), &[]);
            pass.set_bind_group(1, Some(&depth_viz_bg), &[]);
            pass.draw(0..3, 0..1);
        } else {
            // Geometry pass (depth read-only)
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("R-5-color"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.06, g: 0.06, b: 0.08, a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            let pipeline = match self.render_mode {
                0x04 => &self.render.normals_pipeline,
                _ => &self.render.color_pipeline,
            };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, Some(&self.render.camera_bind_group), &[]);
            pass.set_bind_group(1, Some(&self.render.vertex_bind_group), &[]);
            pass.set_bind_group(2, Some(self.pool.scene_global_bind_group()), &[]);
            pass.set_index_buffer(
                self.pool.index_buffer().slice(..),
                wgpu::IndexFormat::Uint32,
            );
            self.draw_all_slots(&mut pass);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        surface_texture.present();

        let frame_t1 = js_sys::Date::now();
        self.timing_depth_ms = (after_depth - frame_t0) as f32;
        self.timing_color_ms = (frame_t1 - after_depth) as f32;
        self.timing_total_ms = (frame_t1 - frame_t0) as f32;

        // Reset per-frame diag counters (currently 0 — compute only runs at load time)
        self.diag_summary_rebuilds = 0;
        self.diag_mesh_rebuilds = 0;

        self.frame_index += 1;
        Ok(())
    }

    /// F8: Lazily allocate wireframe buffers + pass and dispatch build_wireframe.
    /// Called on first wireframe render. Subsequent calls are no-ops.
    fn ensure_wireframe(&mut self) {
        if self.build_wireframe_pass.is_some() {
            return;
        }
        // Allocate wireframe buffers (~128 MB)
        self.pool.ensure_wireframe_buffers(&self.device);

        // Create the build_wireframe compute pass
        let pass = passes::build_wireframe::BuildWireframePass::new(
            &self.device,
            self.pool.index_pool_buf(),
            self.pool.mesh_offset_table_buf(),
            self.pool.wire_index_pool(),
            self.pool.wire_indirect_buf(),
        );

        // Dispatch immediately to populate wireframe indices from current mesh data
        let mut encoder = self.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor { label: Some("build-wireframe-lazy") },
        );
        pass.dispatch(&mut encoder, self.resident_count);
        self.queue.submit(std::iter::once(encoder.finish()));

        self.build_wireframe_pass = Some(pass);
    }

    /// Issue indirect draw for all resident slots (triangle mesh).
    ///
    /// ORDERING (F10): Reads indirect_draw_buf written by build_indirect
    /// (dispatched in load_test_scene or future per-frame compute).
    /// Safe because WebGPU queue ordering guarantees prior submissions complete.
    ///
    /// F9: Loops per-slot because multi_draw_indexed_indirect is not available
    /// on the WebGPU backend. At 1024 slots this is ~0.1ms CPU overhead.
    /// When wgpu::Features::MULTI_DRAW_INDIRECT becomes available for WebGPU,
    /// replace with: pass.multi_draw_indexed_indirect(buf, 0, count);
    fn draw_all_slots(&self, pass: &mut wgpu::RenderPass<'_>) {
        let indirect_buf = self.pool.indirect_buffer();
        for slot in 0..self.resident_count {
            pass.draw_indexed_indirect(indirect_buf, slot as u64 * 20);
        }
    }

    /// Issue indirect draw for all resident slots (wireframe edges).
    fn draw_all_slots_wire(&self, pass: &mut wgpu::RenderPass<'_>) {
        let indirect_buf = self.pool.wire_indirect_buf();
        for slot in 0..self.resident_count {
            pass.draw_indexed_indirect(indirect_buf, slot as u64 * 20);
        }
    }

    /// Rebuild meshes for all chunks — three-pass GPU pipeline or CPU upload.
    /// Also initializes visibility and builds indirect draw args.
    fn rebuild_meshes(&mut self, chunks: &[scene::ChunkData]) {
        let resident_count = self.resident_count;

        self.pool.init_visibility(&self.queue, resident_count);

        if self.use_cpu_mesh {
            // CPU path: run CPU mesher, compute prefix sum on CPU, upload at variable offsets
            log("Using CPU mesh path");
            let mut quad_counts = Vec::with_capacity(chunks.len());
            let mut results = Vec::with_capacity(chunks.len());

            // Pass 1 (CPU): mesh all chunks, collect counts
            for chunk in chunks {
                let pal_words = chunk.palette.as_words();
                let bpe = scene::IndexBufBuilder::bits_per_entry(chunk.palette.len());
                let idx_words = chunk.index_buf.pack(bpe);
                let meta_val = scene::IndexBufBuilder::palette_meta(chunk.palette.len());
                let result = mesh_cpu::mesh_rebuild_cpu(
                    chunk.occupancy.as_words(),
                    &pal_words,
                    &idx_words,
                    meta_val,
                    [chunk.coord.x, chunk.coord.y, chunk.coord.z],
                );
                quad_counts.push(result.quad_count);
                results.push(result);
            }

            // Pass 2 (CPU): prefix sum → offsets (5 u32 per slot)
            let mut offset_table = vec![0u32; pool::MAX_SLOTS as usize * 5];
            let mut vert_offset = 0u32;
            let mut idx_offset = 0u32;
            for (slot, &qc) in quad_counts.iter().enumerate() {
                let vc = qc * 4;
                let ic = qc * 6;
                let base = slot * 5;
                offset_table[base] = vert_offset;
                offset_table[base + 1] = vc;
                offset_table[base + 2] = idx_offset;
                offset_table[base + 3] = ic;
                offset_table[base + 4] = 0; // write_counter
                vert_offset += vc;
                idx_offset += ic;
            }
            log(&format!(
                "  CPU totals: {} vertices, {} indices across {} chunks",
                vert_offset, idx_offset, chunks.len(),
            ));

            // Upload mesh_offset_table
            self.queue.write_buffer(
                self.pool.mesh_offset_table_buf(),
                0,
                bytemuck::cast_slice(&offset_table),
            );

            // Pass 3 (CPU): upload vertices/indices at computed offsets
            for (slot, result) in results.iter().enumerate() {
                let base = slot * 5;
                let vo = offset_table[base];
                let vc = offset_table[base + 1];
                let io = offset_table[base + 2];
                let ic = offset_table[base + 3];
                if vc == 0 { continue; }
                let vert_bytes = (vc * pool::VERTEX_BYTES) as usize;
                let idx_bytes = (ic * pool::INDEX_BYTES) as usize;
                self.queue.write_buffer(
                    self.pool.vertex_pool_buf(),
                    vo as u64 * pool::VERTEX_BYTES as u64,
                    &result.vertices[..vert_bytes],
                );
                self.queue.write_buffer(
                    self.pool.index_pool_buf(),
                    io as u64 * pool::INDEX_BYTES as u64,
                    bytemuck::cast_slice(&result.indices[..ic as usize]),
                );
                log(&format!(
                    "  CPU mesh slot {}: {} verts @{}, {} indices @{}",
                    slot, vc, vo, ic, io,
                ));
            }

            // Build indirect from offset table
            let mut encoder = self.device.create_command_encoder(
                &wgpu::CommandEncoderDescriptor { label: Some("cpu-mesh-indirect") },
            );
            self.build_indirect_pass.dispatch(&mut encoder, resident_count);
            self.queue.submit(std::iter::once(encoder.finish()));
        } else {
            // GPU three-pass pipeline: Count → Prefix Sum → Write

            // Zero mesh_counts before counting
            let zeros = vec![0u8; pool::MESH_COUNTS_ENTRY_BYTES as usize * resident_count as usize];
            self.queue.write_buffer(self.pool.mesh_counts_buf(), 0, &zeros);

            // Zero draw_meta (reused as per-slot write counters in Pass 3)
            let dm_zeros = vec![0u8; pool::DRAW_META_BYTES as usize * resident_count as usize];
            self.queue.write_buffer(self.pool.draw_meta_buf(), 0, &dm_zeros);

            let mut encoder = self.device.create_command_encoder(
                &wgpu::CommandEncoderDescriptor { label: Some("r1-three-pass") },
            );

            // Pass 1: Count quads per slot
            self.mesh_count_pass.dispatch(
                &mut encoder,
                self.pool.mesh_count_bind_group(),
                resident_count,
            );

            // Pass 2: Prefix sum → offset table
            self.prefix_sum_pass.dispatch(
                &mut encoder,
                self.pool.prefix_sum_bind_group(),
            );

            // Pass 3: Write vertices/indices at computed offsets
            self.mesh_pass.dispatch(
                &mut encoder,
                self.pool.mesh_compute_bind_group(),
                resident_count,
            );

            // Build indirect draw args from offset table
            self.build_indirect_pass.dispatch(&mut encoder, resident_count);

            self.queue.submit(std::iter::once(encoder.finish()));
        }

        // Force wireframe rebuild on next activation
        self.build_wireframe_pass = None;
    }
}

#[cfg(target_arch = "wasm32")]
fn log(msg: &str) {
    web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(&format!("[wasm_renderer] {msg}")));
}
