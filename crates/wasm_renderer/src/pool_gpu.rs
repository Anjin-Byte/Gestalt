//! GPU-resident chunk pool — buffer allocation, bind groups, and upload.
//!
//! WASM-only. Owns all wgpu::Buffers for the 1024-slot chunk pool.
//! CPU-side slot management lives in `pool::SlotAllocator` (platform-independent).

use crate::pool::*;

/// Owns all GPU buffers for the chunk pool.
///
/// Buffer memory layout uses the constants from `pool.rs`. Each per-slot buffer
/// is a contiguous array indexed by `slot * PER_SLOT_SIZE`.
pub struct ChunkPool {
    pub(crate) allocator: SlotAllocator,

    // ── Per-slot data plane (authoritative) ──
    pub(crate) occupancy_atlas: wgpu::Buffer,
    pub(crate) palette_buf: wgpu::Buffer,
    pub(crate) coord_buf: wgpu::Buffer,
    pub(crate) version_buf: wgpu::Buffer,

    // ── Per-slot derived (GPU-produced) ──
    pub(crate) flags_buf: wgpu::Buffer,
    pub(crate) summary_buf: wgpu::Buffer,
    pub(crate) aabb_buf: wgpu::Buffer,
    pub(crate) draw_meta_buf: wgpu::Buffer, // retained for CPU mesh path diagnostics

    // ── Mesh pool (variable allocation) ──
    pub(crate) vertex_pool: wgpu::Buffer,
    pub(crate) index_pool: wgpu::Buffer,
    pub(crate) mesh_counts_buf: wgpu::Buffer,      // Pass 1 output: per-slot quad count
    pub(crate) mesh_offset_table: wgpu::Buffer,     // Pass 2 output: per-slot offsets (vec4u)
    pub(crate) mesh_total_buf: wgpu::Buffer,        // Pass 2 output: total verts + indices (2 u32)

    // ── Wireframe (F8: lazy allocation — None until wireframe mode first activated) ──
    pub(crate) wire_index_pool: Option<wgpu::Buffer>,
    pub(crate) wire_indirect_buf: Option<wgpu::Buffer>,

    // ── Per-slot material index buffer (variable allocation) + metadata ──
    pub(crate) index_buf_pool: wgpu::Buffer,
    pub(crate) palette_meta_buf: wgpu::Buffer,
    pub(crate) index_buf_alloc: IndexBufAllocator,

    // ── Per-slot visibility (R-4 output) ──
    pub(crate) visibility_buf: wgpu::Buffer,
    pub(crate) pass1_visibility_buf: wgpu::Buffer,

    // ── Scene-global ──
    pub(crate) scene_params_buf: wgpu::Buffer,
    pub(crate) material_table: wgpu::Buffer,
    pub(crate) indirect_draw_buf: wgpu::Buffer,

    // ── DDA slot table (coord→slot lookup for GI traversal) ──
    pub(crate) slot_table_buf: wgpu::Buffer,
    pub(crate) slot_table_params_buf: wgpu::Buffer,

    // ── Bind group layouts (read-only, for render stages) ──
    pub(crate) chunk_meta_layout: wgpu::BindGroupLayout,
    pub(crate) mesh_draw_layout: wgpu::BindGroupLayout,
    pub(crate) scene_global_layout: wgpu::BindGroupLayout,

    // ── Bind groups (read-only, for render stages) ──
    pub(crate) chunk_meta_bind_group: wgpu::BindGroup,
    pub(crate) mesh_draw_bind_group: wgpu::BindGroup,
    pub(crate) scene_global_bind_group: wgpu::BindGroup,

    // ── Compute bind groups (I-3 summary rebuild) ──
    pub(crate) summary_compute_layout: wgpu::BindGroupLayout,
    pub(crate) summary_compute_bind_group: wgpu::BindGroup,

    // ── Compute bind groups (R-1 mesh count pass) ──
    pub(crate) mesh_count_layout: wgpu::BindGroupLayout,
    pub(crate) mesh_count_bind_group: wgpu::BindGroup,

    // ── Compute bind groups (R-1 prefix sum pass) ──
    pub(crate) prefix_sum_layout: wgpu::BindGroupLayout,
    pub(crate) prefix_sum_bind_group: wgpu::BindGroup,

    // ── Compute bind groups (R-1 mesh write pass) ──
    pub(crate) mesh_compute_layout: wgpu::BindGroupLayout,
    pub(crate) mesh_compute_bind_group: wgpu::BindGroup,
}

impl ChunkPool {
    /// Create the pool, allocating all GPU buffers and bind groups.
    pub fn new(device: &wgpu::Device) -> Self {
        let allocator = SlotAllocator::new();

        // ── Buffer creation ──

        let occupancy_atlas = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk-occupancy-atlas"),
            size: TOTAL_OCCUPANCY_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let palette_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk-palette"),
            size: TOTAL_PALETTE_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let coord_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk-coord"),
            size: COORD_BYTES as u64 * MAX_SLOTS as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let version_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk-version"),
            size: VERSION_BYTES as u64 * MAX_SLOTS as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let flags_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk-flags"),
            size: FLAGS_BYTES as u64 * MAX_SLOTS as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let summary_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk-summary"),
            size: SUMMARY_BYTES_PER_SLOT as u64 * MAX_SLOTS as u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let aabb_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk-aabb"),
            size: AABB_BYTES as u64 * MAX_SLOTS as u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let draw_meta_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk-draw-meta"),
            size: DRAW_META_BYTES as u64 * MAX_SLOTS as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let vertex_pool = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vertex-pool"),
            size: TOTAL_VERTEX_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let index_pool = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("index-pool"),
            size: TOTAL_INDEX_BYTES,
            usage: wgpu::BufferUsages::INDEX
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        // ── Variable mesh pool buffers ──

        let mesh_counts_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh-counts"),
            size: MESH_COUNTS_ENTRY_BYTES as u64 * MAX_SLOTS as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mesh_offset_table = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh-offset-table"),
            size: MESH_OFFSET_ENTRY_BYTES as u64 * MAX_SLOTS as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mesh_total_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh-total"),
            size: 8, // 2 × u32 (total_vertices, total_indices)
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let scene_params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene-params"),
            size: SCENE_PARAMS_BYTES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let material_table = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("material-table"),
            size: TOTAL_MATERIAL_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let index_buf_pool = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk-index-buf-pool"),
            size: INDEX_BUF_POOL_CAPACITY,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let palette_meta_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("palette-meta"),
            size: TOTAL_PALETTE_META_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let visibility_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("visibility"),
            size: 4 * MAX_SLOTS as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pass1_visibility_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pass1-visibility"),
            size: 4 * MAX_SLOTS as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // F8: Wireframe buffers are NOT allocated here. They are created lazily
        // when wireframe mode is first activated, saving ~128 MB GPU memory.
        // See: pool.ensure_wireframe_buffers()

        let indirect_draw_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("indirect-draw"),
            size: TOTAL_INDIRECT_BYTES,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── DDA slot table (coord→slot for GI traversal) ──

        let slot_table_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("slot-table"),
            size: SLOT_TABLE_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let slot_table_params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("slot-table-params"),
            size: SLOT_TABLE_PARAMS_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Bind group layouts ──

        // Group 0 — Chunk Metadata (7 bindings, read-only for render stages)
        // Compute passes that need write access will create their own layouts.
        let chunk_meta_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("chunk-meta-layout"),
                entries: &[
                    storage_entry(0, true), // occupancy_atlas
                    storage_entry(1, true), // palette
                    storage_entry(2, true), // coord
                    storage_entry(3, true), // version
                    storage_entry(4, true), // flags
                    storage_entry(5, true), // summary
                    storage_entry(6, true), // aabb
                ],
            });

        // Group 1 — Mesh + Draw (4 bindings, read-only for render stages)
        let mesh_draw_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("mesh-draw-layout"),
                entries: &[
                    storage_entry(0, true), // vertex_pool
                    storage_entry(1, true), // index_pool
                    storage_entry(2, true), // draw_meta
                    storage_entry(3, true), // indirect_draw
                ],
            });

        // Group 2 — Scene Global (1 binding)
        let scene_global_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("scene-global-layout"),
                entries: &[
                    storage_entry(0, true), // material_table
                ],
            });

        // ── Bind groups ──

        let chunk_meta_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("chunk-meta-bg"),
            layout: &chunk_meta_layout,
            entries: &[
                buf_binding(0, &occupancy_atlas),
                buf_binding(1, &palette_buf),
                buf_binding(2, &coord_buf),
                buf_binding(3, &version_buf),
                buf_binding(4, &flags_buf),
                buf_binding(5, &summary_buf),
                buf_binding(6, &aabb_buf),
            ],
        });

        let mesh_draw_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh-draw-bg"),
            layout: &mesh_draw_layout,
            entries: &[
                buf_binding(0, &vertex_pool),
                buf_binding(1, &index_pool),
                buf_binding(2, &draw_meta_buf),
                buf_binding(3, &indirect_draw_buf),
            ],
        });

        let scene_global_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene-global-bg"),
            layout: &scene_global_layout,
            entries: &[buf_binding(0, &material_table)],
        });

        // ── I-3 Summary Compute bind group (COMPUTE-only, with write access) ──

        let summary_compute_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("summary-compute-layout"),
                entries: &[
                    compute_storage_entry(0, true),  // occupancy_atlas (read)
                    compute_storage_entry(1, true),  // palette (read)
                    compute_storage_entry(2, true),  // coord (read)
                    compute_storage_entry(3, true),  // material_table (read)
                    compute_storage_entry(4, false), // summary (read-write)
                    compute_storage_entry(5, false), // flags (read-write)
                    compute_storage_entry(6, false), // aabb (read-write)
                    // binding 7: scene_params (uniform — not storage, different limit)
                    wgpu::BindGroupLayoutEntry {
                        binding: 7,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let summary_compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("summary-compute-bg"),
            layout: &summary_compute_layout,
            entries: &[
                buf_binding(0, &occupancy_atlas),
                buf_binding(1, &palette_buf),
                buf_binding(2, &coord_buf),
                buf_binding(3, &material_table),
                buf_binding(4, &summary_buf),
                buf_binding(5, &flags_buf),
                buf_binding(6, &aabb_buf),
                buf_binding(7, &scene_params_buf),
            ],
        });

        // ── R-1 Pass 1: Mesh Count bind group (count quads per slot) ──

        let mesh_count_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("mesh-count-layout"),
                entries: &[
                    compute_storage_entry(0, true),  // occupancy_atlas (read)
                    compute_storage_entry(1, true),  // palette (read)
                    compute_storage_entry(2, true),  // coord (read)
                    compute_storage_entry(3, false), // mesh_counts (read-write, atomic)
                    compute_storage_entry(4, true),  // index_buf_pool (read)
                    compute_storage_entry(5, true),  // palette_meta (read)
                ],
            });

        let mesh_count_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh-count-bg"),
            layout: &mesh_count_layout,
            entries: &[
                buf_binding(0, &occupancy_atlas),
                buf_binding(1, &palette_buf),
                buf_binding(2, &coord_buf),
                buf_binding(3, &mesh_counts_buf),
                buf_binding(4, &index_buf_pool),
                buf_binding(5, &palette_meta_buf),
            ],
        });

        // ── R-1 Pass 2: Prefix Sum bind group ──

        let prefix_sum_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("prefix-sum-layout"),
                entries: &[
                    compute_storage_entry(0, true),  // mesh_counts (read)
                    compute_storage_entry(1, false), // mesh_offset_table (read-write)
                    compute_storage_entry(2, false), // mesh_total (read-write)
                ],
            });

        let prefix_sum_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("prefix-sum-bg"),
            layout: &prefix_sum_layout,
            entries: &[
                buf_binding(0, &mesh_counts_buf),
                buf_binding(1, &mesh_offset_table),
                buf_binding(2, &mesh_total_buf),
            ],
        });

        // ── R-1 Pass 3: Mesh Write bind group (write vertices/indices at computed offsets) ──

        let mesh_compute_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("mesh-compute-layout"),
                entries: &[
                    compute_storage_entry(0, true),  // occupancy_atlas (read)
                    compute_storage_entry(1, true),  // palette (read)
                    compute_storage_entry(2, true),  // coord (read)
                    compute_storage_entry(3, false), // vertex_pool (read-write)
                    compute_storage_entry(4, false), // index_pool (read-write)
                    compute_storage_entry(5, false), // mesh_offset_table (read-write, atomic write counter)
                    compute_storage_entry(6, true),  // index_buf_pool (read)
                    compute_storage_entry(7, true),  // palette_meta (read)
                    // binding 8: scene_params (uniform — not storage, different limit)
                    wgpu::BindGroupLayoutEntry {
                        binding: 8,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let mesh_compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh-compute-bg"),
            layout: &mesh_compute_layout,
            entries: &[
                buf_binding(0, &occupancy_atlas),
                buf_binding(1, &palette_buf),
                buf_binding(2, &coord_buf),
                buf_binding(3, &vertex_pool),
                buf_binding(4, &index_pool),
                buf_binding(5, &mesh_offset_table),
                buf_binding(6, &index_buf_pool),
                buf_binding(7, &palette_meta_buf),
                buf_binding(8, &scene_params_buf),
            ],
        });

        let total_bytes = TOTAL_OCCUPANCY_BYTES
            + TOTAL_PALETTE_BYTES
            + TOTAL_INDEX_BUF_BYTES
            + TOTAL_PALETTE_META_BYTES
            + (COORD_BYTES as u64 * MAX_SLOTS as u64)
            + (VERSION_BYTES as u64 * MAX_SLOTS as u64)
            + (FLAGS_BYTES as u64 * MAX_SLOTS as u64)
            + (SUMMARY_BYTES_PER_SLOT as u64 * MAX_SLOTS as u64)
            + (AABB_BYTES as u64 * MAX_SLOTS as u64)
            + (DRAW_META_BYTES as u64 * MAX_SLOTS as u64)
            + TOTAL_VERTEX_BYTES
            + TOTAL_INDEX_BYTES
            + (MESH_COUNTS_ENTRY_BYTES as u64 * MAX_SLOTS as u64)
            + (MESH_OFFSET_ENTRY_BYTES as u64 * MAX_SLOTS as u64)
            + 8 // mesh_total
            + TOTAL_MATERIAL_BYTES
            + TOTAL_INDIRECT_BYTES;
        web_sys::console::log_1(
            &wasm_bindgen::JsValue::from_str(&format!(
                "[wasm_renderer] ChunkPool allocated: {} MB ({} slots)",
                total_bytes / (1024 * 1024),
                MAX_SLOTS
            )),
        );

        Self {
            allocator,
            occupancy_atlas,
            palette_buf,
            coord_buf,
            version_buf,
            flags_buf,
            summary_buf,
            aabb_buf,
            draw_meta_buf,
            vertex_pool,
            index_pool,
            mesh_counts_buf,
            mesh_offset_table,
            mesh_total_buf,
            wire_index_pool: None,
            wire_indirect_buf: None,
            index_buf_pool,
            palette_meta_buf,
            index_buf_alloc: IndexBufAllocator::new(),
            visibility_buf,
            pass1_visibility_buf,
            scene_params_buf,
            material_table,
            indirect_draw_buf,
            slot_table_buf,
            slot_table_params_buf,
            chunk_meta_layout,
            mesh_draw_layout,
            scene_global_layout,
            chunk_meta_bind_group,
            mesh_draw_bind_group,
            scene_global_bind_group,
            summary_compute_layout,
            summary_compute_bind_group,
            mesh_count_layout,
            mesh_count_bind_group,
            prefix_sum_layout,
            prefix_sum_bind_group,
            mesh_compute_layout,
            mesh_compute_bind_group,
        }
    }

    // ── Slot management ──

    /// Allocate a slot for a chunk coordinate.
    pub fn alloc_slot(&mut self, coord: ChunkCoord) -> Result<u32, AllocError> {
        self.allocator.alloc(coord)
    }

    /// Deallocate a slot. Writes version=0 to GPU to mark the slot as invalid.
    pub fn dealloc_slot(
        &mut self,
        slot: u32,
        queue: &wgpu::Queue,
    ) -> Result<ChunkCoord, DeallocError> {
        let coord = self.allocator.dealloc(slot)?;
        // Zero the version on GPU so no consumer reads stale data
        queue.write_buffer(
            &self.version_buf,
            slot as u64 * VERSION_BYTES as u64,
            &[0u8; VERSION_BYTES as usize],
        );
        Ok(coord)
    }

    // ── Upload methods (I-2 implementation) ──

    /// Upload all authoritative data for a chunk: occupancy, palette, index_buf, palette_meta, coord.
    /// Sets version to 1 and marks the slot for summary rebuild.
    pub fn upload_chunk(
        &self,
        queue: &wgpu::Queue,
        slot: u32,
        coord: ChunkCoord,
        occupancy: &[u32],
        palette: &[u32],
        index_buf_words: &[u32],
        index_buf_word_offset: u32,
        palette_meta_word0: u32,
    ) {
        assert!(slot < MAX_SLOTS, "slot {slot} out of range");
        assert!(
            occupancy.len() == OCCUPANCY_WORDS_PER_SLOT as usize,
            "occupancy must be exactly {} words, got {}",
            OCCUPANCY_WORDS_PER_SLOT,
            occupancy.len()
        );
        assert!(
            palette.len() <= PALETTE_WORDS_PER_SLOT as usize,
            "palette exceeds {} words",
            PALETTE_WORDS_PER_SLOT
        );

        self.upload_occupancy(queue, slot, occupancy);
        self.upload_palette(queue, slot, palette);

        // Write per-voxel palette index buffer at variable offset
        if !index_buf_words.is_empty() {
            queue.write_buffer(
                &self.index_buf_pool,
                index_buf_word_offset as u64 * 4,
                bytemuck::cast_slice(index_buf_words),
            );
        }

        // Write palette metadata: 2 × u32 per slot
        //   [0]: palette_size | bpe | reserved
        //   [1]: index_buf_word_offset
        queue.write_buffer(
            &self.palette_meta_buf,
            slot as u64 * PALETTE_META_BYTES as u64,
            bytemuck::cast_slice(&[palette_meta_word0, index_buf_word_offset]),
        );

        // Write coord as vec4i (x, y, z, 0)
        let coord_data: [i32; 4] = [coord.x, coord.y, coord.z, 0];
        queue.write_buffer(
            &self.coord_buf,
            slot as u64 * COORD_BYTES as u64,
            bytemuck::cast_slice(&coord_data),
        );

        // Set version = 1
        let version: [u32; 1] = [1];
        queue.write_buffer(
            &self.version_buf,
            slot as u64 * VERSION_BYTES as u64,
            bytemuck::cast_slice(&version),
        );

        // Set stale_summary flag (bit 5) so I-3 knows to rebuild
        let flags: [u32; 1] = [1 << 5];
        queue.write_buffer(
            &self.flags_buf,
            slot as u64 * FLAGS_BYTES as u64,
            bytemuck::cast_slice(&flags),
        );
    }

    /// Upload occupancy data only (for partial updates).
    pub fn upload_occupancy(&self, queue: &wgpu::Queue, slot: u32, occupancy: &[u32]) {
        assert!(slot < MAX_SLOTS, "slot {slot} out of range");
        assert!(
            occupancy.len() == OCCUPANCY_WORDS_PER_SLOT as usize,
            "occupancy must be exactly {} words",
            OCCUPANCY_WORDS_PER_SLOT
        );
        queue.write_buffer(
            &self.occupancy_atlas,
            slot as u64 * OCCUPANCY_BYTES_PER_SLOT as u64,
            bytemuck::cast_slice(occupancy),
        );
    }

    /// Upload palette data only.
    pub fn upload_palette(&self, queue: &wgpu::Queue, slot: u32, palette: &[u32]) {
        assert!(slot < MAX_SLOTS, "slot {slot} out of range");
        assert!(
            palette.len() <= PALETTE_WORDS_PER_SLOT as usize,
            "palette exceeds {} words",
            PALETTE_WORDS_PER_SLOT
        );
        if palette.is_empty() {
            return;
        }
        queue.write_buffer(
            &self.palette_buf,
            slot as u64 * PALETTE_BYTES_PER_SLOT as u64,
            bytemuck::cast_slice(palette),
        );
    }

    /// Upload vertex and index data for a mesh rebuild result.
    pub fn upload_mesh(
        &self,
        queue: &wgpu::Queue,
        slot: u32,
        vertices: &[u8],
        indices: &[u8],
    ) {
        assert!(slot < MAX_SLOTS, "slot {slot} out of range");
        let max_vert_bytes = (MAX_VERTS_PER_CHUNK * VERTEX_BYTES) as usize;
        let max_idx_bytes = (MAX_INDICES_PER_CHUNK * INDEX_BYTES) as usize;
        assert!(
            vertices.len() <= max_vert_bytes,
            "vertex data {} bytes exceeds max {}",
            vertices.len(),
            max_vert_bytes
        );
        assert!(
            indices.len() <= max_idx_bytes,
            "index data {} bytes exceeds max {}",
            indices.len(),
            max_idx_bytes
        );

        if !vertices.is_empty() {
            queue.write_buffer(
                &self.vertex_pool,
                slot as u64 * max_vert_bytes as u64,
                vertices,
            );
        }
        if !indices.is_empty() {
            queue.write_buffer(
                &self.index_pool,
                slot as u64 * max_idx_bytes as u64,
                indices,
            );
        }
    }

    /// Upload draw metadata for a single slot from CPU-side data.
    pub fn upload_draw_meta(&self, queue: &wgpu::Queue, slot: u32, meta: &DrawMeta) {
        assert!(slot < MAX_SLOTS, "slot {slot} out of range");
        queue.write_buffer(
            &self.draw_meta_buf,
            slot as u64 * DRAW_META_BYTES as u64,
            bytemuck::bytes_of(meta),
        );
    }

    /// Upload material table entries (scene-global, not per-slot).
    pub fn upload_materials(&self, queue: &wgpu::Queue, materials: &[u8]) {
        assert!(
            materials.len() as u64 <= TOTAL_MATERIAL_BYTES,
            "material data {} bytes exceeds max {}",
            materials.len(),
            TOTAL_MATERIAL_BYTES
        );
        if !materials.is_empty() {
            queue.write_buffer(&self.material_table, 0, materials);
        }
    }

    // ── Accessors ──

    pub fn chunk_meta_layout(&self) -> &wgpu::BindGroupLayout {
        &self.chunk_meta_layout
    }
    pub fn mesh_draw_layout(&self) -> &wgpu::BindGroupLayout {
        &self.mesh_draw_layout
    }
    pub fn scene_global_layout(&self) -> &wgpu::BindGroupLayout {
        &self.scene_global_layout
    }
    pub fn chunk_meta_bind_group(&self) -> &wgpu::BindGroup {
        &self.chunk_meta_bind_group
    }
    pub fn mesh_draw_bind_group(&self) -> &wgpu::BindGroup {
        &self.mesh_draw_bind_group
    }
    pub fn scene_global_bind_group(&self) -> &wgpu::BindGroup {
        &self.scene_global_bind_group
    }
    pub fn index_buffer(&self) -> &wgpu::Buffer {
        &self.index_pool
    }
    pub fn indirect_buffer(&self) -> &wgpu::Buffer {
        &self.indirect_draw_buf
    }
    pub fn allocator(&self) -> &SlotAllocator {
        &self.allocator
    }
    pub fn allocator_mut(&mut self) -> &mut SlotAllocator {
        &mut self.allocator
    }
    pub fn visibility_buf(&self) -> &wgpu::Buffer {
        &self.visibility_buf
    }
    pub fn aabb_buf(&self) -> &wgpu::Buffer {
        &self.aabb_buf
    }
    pub fn flags_buf(&self) -> &wgpu::Buffer {
        &self.flags_buf
    }

    /// Patch all indirect draw entries to instance_count=1, making all slots
    /// visible for the depth prepass. Each entry is 20 bytes (5 × u32);
    /// instance_count is at offset 4 within each entry.
    pub fn force_all_visible(&self, queue: &wgpu::Queue, slot_count: u32) {
        let one = [1u32];
        let one_bytes = bytemuck::cast_slice(&one);
        for slot in 0..slot_count {
            queue.write_buffer(
                &self.indirect_draw_buf,
                slot as u64 * 20 + 4,
                one_bytes,
            );
        }
    }

    /// Set visibility to 1 (visible) for the first `slot_count` slots.
    /// Call after loading/uploading chunks so the first frame's indirect draw works.
    pub fn init_visibility(&self, queue: &wgpu::Queue, slot_count: u32) {
        let ones = vec![1u32; slot_count as usize];
        queue.write_buffer(
            &self.visibility_buf,
            0,
            bytemuck::cast_slice(&ones),
        );
    }
    pub fn summary_compute_layout(&self) -> &wgpu::BindGroupLayout {
        &self.summary_compute_layout
    }
    pub fn summary_compute_bind_group(&self) -> &wgpu::BindGroup {
        &self.summary_compute_bind_group
    }
    pub fn mesh_count_layout(&self) -> &wgpu::BindGroupLayout {
        &self.mesh_count_layout
    }
    pub fn mesh_count_bind_group(&self) -> &wgpu::BindGroup {
        &self.mesh_count_bind_group
    }
    pub fn prefix_sum_layout(&self) -> &wgpu::BindGroupLayout {
        &self.prefix_sum_layout
    }
    pub fn prefix_sum_bind_group(&self) -> &wgpu::BindGroup {
        &self.prefix_sum_bind_group
    }
    pub fn mesh_compute_layout(&self) -> &wgpu::BindGroupLayout {
        &self.mesh_compute_layout
    }
    pub fn mesh_compute_bind_group(&self) -> &wgpu::BindGroup {
        &self.mesh_compute_bind_group
    }
    pub fn mesh_counts_buf(&self) -> &wgpu::Buffer {
        &self.mesh_counts_buf
    }
    pub fn mesh_offset_table_buf(&self) -> &wgpu::Buffer {
        &self.mesh_offset_table
    }
    pub fn pass1_visibility_buf(&self) -> &wgpu::Buffer {
        &self.pass1_visibility_buf
    }
    pub fn scene_params_buf(&self) -> &wgpu::Buffer {
        &self.scene_params_buf
    }
    pub fn reset_index_buf_alloc(&mut self) {
        self.index_buf_alloc.reset();
    }
    pub fn alloc_index_buf(&mut self, words: u32) -> u32 {
        self.index_buf_alloc.alloc(words)
    }

    /// Upload scene params: grid_origin (xyz) + voxel_size (w) packed as vec4f.
    pub fn upload_scene_params(&self, queue: &wgpu::Queue, grid_origin: [f32; 3], voxel_size: f32) {
        let data = [grid_origin[0], grid_origin[1], grid_origin[2], voxel_size];
        queue.write_buffer(&self.scene_params_buf, 0, bytemuck::cast_slice(&data));
    }
    pub fn draw_meta_buf(&self) -> &wgpu::Buffer {
        &self.draw_meta_buf
    }
    pub fn vertex_pool_buf(&self) -> &wgpu::Buffer {
        &self.vertex_pool
    }
    pub fn index_pool_buf(&self) -> &wgpu::Buffer {
        &self.index_pool
    }
    /// F8: Lazy wireframe buffer allocation. Creates ~128 MB of buffers on first call.
    /// Subsequent calls are no-ops. Call before dispatching build_wireframe or drawing wireframe.
    pub fn ensure_wireframe_buffers(&mut self, device: &wgpu::Device) {
        if self.wire_index_pool.is_some() {
            return;
        }
        self.wire_index_pool = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wire-index-pool"),
            size: TOTAL_WIRE_INDEX_BYTES,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        }));
        self.wire_indirect_buf = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wire-indirect"),
            size: TOTAL_INDIRECT_BYTES,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        }));
        web_sys::console::log_1(
            &wasm_bindgen::JsValue::from_str("[wasm_renderer] Wireframe buffers allocated (~128 MB)"),
        );
    }

    pub fn wire_index_pool(&self) -> &wgpu::Buffer {
        self.wire_index_pool.as_ref().expect("wireframe buffers not allocated — call ensure_wireframe_buffers first")
    }
    pub fn wire_indirect_buf(&self) -> &wgpu::Buffer {
        self.wire_indirect_buf.as_ref().expect("wireframe buffers not allocated — call ensure_wireframe_buffers first")
    }
    pub fn has_wireframe_buffers(&self) -> bool {
        self.wire_index_pool.is_some()
    }

    // ── DDA slot table ──

    pub fn slot_table_buf(&self) -> &wgpu::Buffer { &self.slot_table_buf }
    pub fn slot_table_params_buf(&self) -> &wgpu::Buffer { &self.slot_table_params_buf }
    pub fn occupancy_atlas_buf(&self) -> &wgpu::Buffer { &self.occupancy_atlas }
    pub fn palette_buf(&self) -> &wgpu::Buffer { &self.palette_buf }
    pub fn palette_meta_buf(&self) -> &wgpu::Buffer { &self.palette_meta_buf }
    pub fn index_buf_pool_buf(&self) -> &wgpu::Buffer { &self.index_buf_pool }
    pub fn material_table_buf(&self) -> &wgpu::Buffer { &self.material_table }

    /// Rebuild and upload the GPU slot table from the current allocator state.
    /// Call after loading/unloading chunks.
    pub fn upload_slot_table(&self, queue: &wgpu::Queue) {
        // Compute bounding box of all allocated chunk coords
        let mut min_coord = [i32::MAX; 3];
        let mut max_coord = [i32::MIN; 3];
        let mut has_any = false;

        for (_slot, coord) in self.allocator.allocated_slots() {
            has_any = true;
            min_coord[0] = min_coord[0].min(coord.x);
            min_coord[1] = min_coord[1].min(coord.y);
            min_coord[2] = min_coord[2].min(coord.z);
            max_coord[0] = max_coord[0].max(coord.x);
            max_coord[1] = max_coord[1].max(coord.y);
            max_coord[2] = max_coord[2].max(coord.z);
        }

        if !has_any {
            return;
        }

        // Center the origin so coords map into [0, DIM)
        let dim = SLOT_TABLE_DIM as i32;
        let origin = [
            (min_coord[0] + max_coord[0]) / 2 - dim / 2,
            (min_coord[1] + max_coord[1]) / 2 - dim / 2,
            (min_coord[2] + max_coord[2]) / 2 - dim / 2,
        ];

        // Fill table with sentinel
        let mut table = vec![SLOT_TABLE_SENTINEL; SLOT_TABLE_ENTRIES as usize];

        for (slot, coord) in self.allocator.allocated_slots() {
            let lx = coord.x - origin[0];
            let ly = coord.y - origin[1];
            let lz = coord.z - origin[2];
            if lx >= 0 && lx < dim && ly >= 0 && ly < dim && lz >= 0 && lz < dim {
                let idx = lx as u32 + ly as u32 * SLOT_TABLE_DIM + lz as u32 * SLOT_TABLE_DIM * SLOT_TABLE_DIM;
                table[idx as usize] = slot;
            }
        }

        queue.write_buffer(&self.slot_table_buf, 0, bytemuck::cast_slice(&table));

        // Upload params: origin.xyz + dim
        let params: [i32; 4] = [origin[0], origin[1], origin[2], dim];
        queue.write_buffer(&self.slot_table_params_buf, 0, bytemuck::cast_slice(&params));
    }
}

// ── Helpers ──

/// Create a bind group layout entry for a storage buffer.
///
/// Read-only bindings are visible to COMPUTE + VERTEX_FRAGMENT.
/// Read-write bindings are COMPUTE-only (WebGPU forbids writable storage
/// in vertex/fragment without VERTEX_WRITABLE_STORAGE, which isn't standard).
fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: if read_only {
            wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::VERTEX_FRAGMENT
        } else {
            wgpu::ShaderStages::COMPUTE
        },
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Create a bind group layout entry for a COMPUTE-only storage buffer.
/// Used for compute passes that need write access (I-3, R-1, R-4).
fn compute_storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Create a bind group entry binding a whole buffer.
fn buf_binding(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}
