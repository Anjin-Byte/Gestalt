//! Global illumination subsystem.
//!
//! Multiple GI backends coexist behind the [`GiBackend`] trait. The active
//! backend is selected by [`GI_PIPELINE`] and constructed via
//! [`create_backend`]. The `Renderer` holds `Box<dyn GiBackend>` and calls
//! trait methods for lifecycle events, per-frame dispatch, and consumer
//! bind group attachment.
//!
//! See:
//! - `docs/Resident Representation/radiance-cascades-v3-design.md`
//! - `docs/Resident Representation/radiance-cascades-symptoms.md`

pub mod v3;

#[cfg(target_arch = "wasm32")]
pub mod null_backend;
#[cfg(target_arch = "wasm32")]
pub mod v2_legacy;

// ─── Backend selector ──────────────────────────────────────────────────

/// Which GI backend the renderer instantiates at startup.
/// Can be switched at runtime via `Renderer::set_gi_backend`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GiPipeline {
    /// No GI — direct sun + hemisphere ambient only.
    Null,
    /// v2 legacy: screen-space probes, ping-pong cascade atlases.
    V2Legacy,
    /// v3 world-space: volumetric probes, sparse chunk-keyed allocation.
    V3Cascades,
    // V3Hybrid — future
}

/// Default GI backend at startup.
pub const GI_PIPELINE: GiPipeline = GiPipeline::V3Cascades;

// ─── Trait + param structs (wasm32 only — depend on wgpu types) ────────

#[cfg(target_arch = "wasm32")]
pub use backend_types::*;

#[cfg(target_arch = "wasm32")]
mod backend_types {
    use crate::pool;

    /// Per-frame parameters passed to [`GiBackend::dispatch_build`].
    /// Extensible — future backends may need additional fields.
    pub struct GiBuildParams {
        pub gi_enabled: bool,
        pub resident_count: u32,
        pub frame_index: u32,
        pub voxel_scale: f32,
        pub grid_origin: [f32; 3],
        pub camera_proj_inv: glam::Mat4,
        pub camera_view_inv: glam::Mat4,
        pub camera_view_proj: glam::Mat4,
        pub prev_view_proj: glam::Mat4,
        pub screen_width: u32,
        pub screen_height: u32,
    }

    /// Parameters for optional debug visualization rendering.
    pub struct GiDebugParams<'a> {
        pub target: &'a wgpu::TextureView,
        pub depth_view: &'a wgpu::TextureView,
        pub normal_view: &'a wgpu::TextureView,
        pub camera_proj_inv: glam::Mat4,
        pub camera_view_inv: glam::Mat4,
        pub mode: u8, // sub-mode within 0x20..0x27 range
    }

    /// The GI subsystem interface. Every backend implements this trait.
    ///
    /// The `Renderer` holds `Box<dyn GiBackend>` and calls these methods
    /// at well-defined lifecycle points. All GI-specific GPU resources,
    /// bind group layouts, pipelines, and shaders live inside the backend.
    /// The renderer never directly accesses implementation internals.
    ///
    /// # Object safety
    /// The trait is object-safe: no generics, no `Self: Sized` constraints,
    /// all methods use `&self` or `&mut self`.
    pub trait GiBackend {
        /// A new chunk has become resident in the chunk pool.
        /// World-space backends use this to allocate per-chunk probe data.
        /// Screen-space backends can no-op.
        fn on_chunk_resident(
            &mut self,
            queue: &wgpu::Queue,
            chunk_slot: u32,
            coord: pool::ChunkCoord,
        );

        /// A chunk has been evicted from the chunk pool.
        /// World-space backends use this to free per-chunk probe data.
        fn on_chunk_evicted(
            &mut self,
            queue: &wgpu::Queue,
            chunk_slot: u32,
            coord: pool::ChunkCoord,
        );

        /// The scene has been fully cleared (all chunks removed).
        /// Reset all backend state to empty.
        fn on_scene_reset(&mut self, queue: &wgpu::Queue);

        /// All chunk uploads for a batch are complete and the chunk pool's
        /// slot table has been finalized. The backend should rebuild any
        /// internal lookup tables (e.g., chunk_slot → probe_slot mappings).
        fn on_residency_settled(
            &mut self,
            queue: &wgpu::Queue,
            allocator: &pool::SlotAllocator,
        );

        /// Per-frame GI build dispatch. Records compute passes into the
        /// encoder. May also write uniform buffers via `queue`.
        fn dispatch_build(
            &mut self,
            encoder: &mut wgpu::CommandEncoder,
            queue: &wgpu::Queue,
            params: &GiBuildParams,
        );

        /// Handle window resize. Screen-space backends need to recreate
        /// resolution-dependent resources. World-space backends can no-op.
        /// Default implementation is a no-op.
        fn on_resize(
            &mut self,
            _device: &wgpu::Device,
            _queue: &wgpu::Queue,
            _width: u32,
            _height: u32,
            _depth_view: &wgpu::TextureView,
            _normal_view: &wgpu::TextureView,
        ) {
        }

        /// The bind group to attach at group 3 for the color/normals/wireframe
        /// render passes. Called every frame.
        fn consumer_bind_group(&self) -> &wgpu::BindGroup;

        /// The bind group layout for group 3. Called once during
        /// `RenderResources::new()` to compile the color pipeline.
        fn consumer_layout(&self) -> &wgpu::BindGroupLayout;

        /// Complete WGSL source for the PBR consumer shader (`solid_*.wgsl`).
        /// Called once during `RenderResources::new()`. The source must
        /// define `vs_main` and `fs_main` entry points compatible with the
        /// color pipeline layout (groups 0-2 are camera, vertex, material;
        /// group 3 is whatever `consumer_layout()` returns).
        fn consumer_shader_source(&self) -> String;

        /// Optional debug visualization overlay. Returns `true` if it
        /// rendered something, `false` to skip. Default is no-op.
        fn debug_render(
            &mut self,
            _encoder: &mut wgpu::CommandEncoder,
            _queue: &wgpu::Queue,
            _device: &wgpu::Device,
            _params: &GiDebugParams,
        ) -> bool {
            false
        }
    }

    /// Factory: construct the active GI backend based on [`super::GI_PIPELINE`].
    ///
    /// Called once during `Renderer::new()`. The returned backend owns all
    /// GI-specific GPU resources and is the sole interface for GI operations.
    #[allow(unused_variables)]
    pub fn create_backend(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        color_format: wgpu::TextureFormat,
        depth_view: &wgpu::TextureView,
        normal_view: &wgpu::TextureView,
        occupancy_buf: &wgpu::Buffer,
        flags_buf: &wgpu::Buffer,
        slot_table_buf: &wgpu::Buffer,
        material_table_buf: &wgpu::Buffer,
        palette_buf: &wgpu::Buffer,
        palette_meta_buf: &wgpu::Buffer,
        index_buf_pool_buf: &wgpu::Buffer,
        slot_table_params_buf: &wgpu::Buffer,
    ) -> Box<dyn GiBackend> {
        create_backend_for(
            super::GI_PIPELINE,
            device, queue, width, height, color_format,
            depth_view, normal_view,
            occupancy_buf, flags_buf, slot_table_buf,
            material_table_buf, palette_buf, palette_meta_buf,
            index_buf_pool_buf, slot_table_params_buf,
        )
    }

    /// Construct a specific GI backend by enum value. Used by both the
    /// startup factory and the runtime `set_gi_backend` switch.
    #[allow(unused_variables)]
    pub fn create_backend_for(
        pipeline: super::GiPipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        color_format: wgpu::TextureFormat,
        depth_view: &wgpu::TextureView,
        normal_view: &wgpu::TextureView,
        occupancy_buf: &wgpu::Buffer,
        flags_buf: &wgpu::Buffer,
        slot_table_buf: &wgpu::Buffer,
        material_table_buf: &wgpu::Buffer,
        palette_buf: &wgpu::Buffer,
        palette_meta_buf: &wgpu::Buffer,
        index_buf_pool_buf: &wgpu::Buffer,
        slot_table_params_buf: &wgpu::Buffer,
    ) -> Box<dyn GiBackend> {
        match pipeline {
            super::GiPipeline::Null => {
                Box::new(super::null_backend::NullBackend::new(device))
            }
            super::GiPipeline::V3Cascades => {
                Box::new(super::v3::backend::V3Backend::new(
                    device, queue,
                    occupancy_buf, flags_buf, slot_table_buf,
                    material_table_buf, palette_buf, palette_meta_buf,
                    index_buf_pool_buf, slot_table_params_buf,
                ))
            }
            super::GiPipeline::V2Legacy => {
                Box::new(super::v2_legacy::backend::V2LegacyBackend::new(
                    device, queue, width, height, color_format,
                    depth_view, normal_view,
                    occupancy_buf, flags_buf, slot_table_buf,
                    material_table_buf, palette_buf, palette_meta_buf,
                    index_buf_pool_buf, slot_table_params_buf,
                ))
            }
        }
    }
}
