//! v3 world-space GI backend — implements `GiBackend`.
//!
//! Absorbs all v3-specific GPU state from `Renderer`:
//! - `V3CascadeResources` (probe payload SSBO, slot tables, params uniform)
//! - `ProbeSlotAllocator` (chunk coord → v3 probe slot mapping)
//! - `V3CascadePipeline` (compute build pipeline + bind groups)
//! - Consumer bind group (group 3 for `solid_v3.wgsl`)

use crate::gi::{GiBackend, GiBuildParams};
use crate::gi::v3::constants;
use crate::gi::v3::dispatch::V3CascadePipeline;
use crate::gi::v3::probe_slot::ProbeSlotAllocator;
use crate::gi::v3::resources::{V3CascadeParams, V3CascadeResources};
use crate::pool;

fn log(msg: &str) {
    web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(
        &format!("[v3-backend] {msg}"),
    ));
}

pub struct V3Backend {
    resources: V3CascadeResources,
    probe_slots: ProbeSlotAllocator,
    pipeline: V3CascadePipeline,
    world_data_bg: wgpu::BindGroup,
    cascade_data_bg: wgpu::BindGroup,
    consumer_bg: wgpu::BindGroup,
}

impl V3Backend {
    /// Create the v3 world-space backend. Called from the factory in `gi/mod.rs`.
    ///
    /// All chunk pool buffer references are borrowed for bind group creation
    /// only — the backend does not retain refs to the pool.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        occupancy_buf: &wgpu::Buffer,
        flags_buf: &wgpu::Buffer,
        slot_table_buf: &wgpu::Buffer,
        material_table_buf: &wgpu::Buffer,
        palette_buf: &wgpu::Buffer,
        palette_meta_buf: &wgpu::Buffer,
        index_buf_pool_buf: &wgpu::Buffer,
        slot_table_params_buf: &wgpu::Buffer,
    ) -> Self {
        let resources = V3CascadeResources::new(device);
        resources.clear_slot_table(queue);

        let pipeline = V3CascadePipeline::new(device);
        let world_data_bg = pipeline.create_world_data_bind_group(
            device,
            occupancy_buf,
            flags_buf,
            slot_table_buf,
            material_table_buf,
            palette_buf,
            palette_meta_buf,
            index_buf_pool_buf,
            slot_table_params_buf,
        );
        let cascade_data_bg = pipeline.create_cascade_data_bind_group(device, &resources);

        let consumer_bg = resources.create_group3_bind_group(
            device,
            slot_table_buf,
            slot_table_params_buf,
        );

        Self {
            resources,
            probe_slots: ProbeSlotAllocator::new(),
            pipeline,
            world_data_bg,
            cascade_data_bg,
            consumer_bg,
        }
    }
}

impl GiBackend for V3Backend {
    fn on_chunk_resident(
        &mut self,
        _queue: &wgpu::Queue,
        _chunk_slot: u32,
        coord: pool::ChunkCoord,
    ) {
        if let Err(e) = self.probe_slots.alloc(coord) {
            log(&format!(
                "probe slot alloc failed for chunk ({},{},{}): {e:?}",
                coord.x, coord.y, coord.z,
            ));
        }
    }

    fn on_chunk_evicted(
        &mut self,
        _queue: &wgpu::Queue,
        _chunk_slot: u32,
        coord: pool::ChunkCoord,
    ) {
        if let Some(probe_slot) = self.probe_slots.lookup(&coord) {
            if let Err(e) = self.probe_slots.dealloc(probe_slot) {
                log(&format!(
                    "probe slot dealloc failed for chunk ({},{},{}): {e:?}",
                    coord.x, coord.y, coord.z,
                ));
            }
        }
    }

    fn on_scene_reset(&mut self, queue: &wgpu::Queue) {
        self.probe_slots.clear();
        self.resources.clear_slot_table(queue);
    }

    fn on_residency_settled(
        &mut self,
        queue: &wgpu::Queue,
        allocator: &pool::SlotAllocator,
    ) {
        // Build chunk_slot → probe_slot and probe_slot → chunk tables.
        let mut chunk_to_probe =
            vec![constants::V3_PROBE_SENTINEL; pool::MAX_SLOTS as usize];
        let mut probe_to_chunk =
            vec![[0, 0, 0, -1i32]; constants::V3_MAX_PROBE_SLOTS as usize];

        for (chunk_slot, coord) in allocator.allocated_slots() {
            if let Some(probe_slot) = self.probe_slots.lookup(&coord) {
                chunk_to_probe[chunk_slot as usize] = probe_slot;
                probe_to_chunk[probe_slot as usize] =
                    [coord.x, coord.y, coord.z, chunk_slot as i32];
            }
        }

        queue.write_buffer(
            &self.resources.probe_slot_table_buf,
            0,
            bytemuck::cast_slice(&chunk_to_probe),
        );
        queue.write_buffer(
            &self.resources.active_probe_chunks_buf,
            0,
            bytemuck::cast_slice(&probe_to_chunk),
        );
    }

    fn dispatch_build(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        params: &GiBuildParams,
    ) {
        if !params.gi_enabled || params.resident_count == 0 {
            return;
        }

        // Update cascade params uniform.
        let cascade_params = V3CascadeParams {
            grid_origin: params.grid_origin,
            voxel_scale: params.voxel_scale,
            cascade_0_spacing: constants::V3_CASCADE_0_SPACING,
            dirs_per_axis: constants::V3_CASCADE_0_DIRS_PER_AXIS,
            probes_per_chunk_axis: constants::V3_PROBES_PER_CHUNK_AXIS,
            max_probe_slots: constants::V3_MAX_PROBE_SLOTS,
            frame_index: params.frame_index,
            active_probe_slots: self.probe_slots.allocated_count(),
            _pad: [0; 6],
        };
        queue.write_buffer(
            &self.resources.cascade_params_buf,
            0,
            bytemuck::bytes_of(&cascade_params),
        );

        self.pipeline.dispatch(
            encoder,
            &self.world_data_bg,
            &self.cascade_data_bg,
        );
    }

    fn consumer_bind_group(&self) -> &wgpu::BindGroup {
        &self.consumer_bg
    }

    fn consumer_layout(&self) -> &wgpu::BindGroupLayout {
        &self.resources.group3_layout
    }

    fn consumer_shader_source(&self) -> String {
        let cascade_common = include_str!("shaders/cascade_common.wgsl");
        let solid_v3 = include_str!("../../shaders/solid_v3.wgsl");
        format!("{}\n{}", cascade_common, solid_v3)
    }
}
