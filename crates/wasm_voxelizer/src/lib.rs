use js_sys::{Float32Array, Object, Reflect, Uint32Array};
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

use voxelizer::core::{MeshInput, TileSpec, VoxelGridSpec, VoxelizeOpts};
use voxelizer::gpu::{GpuVoxelizer, GpuVoxelizerConfig};
use voxelizer::reference_cpu::voxelize_surface_cpu;

thread_local! {
    static LOG_ENABLED: std::cell::Cell<bool> = std::cell::Cell::new(true);
}

fn log(message: &str) {
    if LOG_ENABLED.with(|enabled| enabled.get()) {
        web_sys::console::log_1(&message.into());
    }
}

#[wasm_bindgen]
pub fn set_log_enabled(enabled: bool) {
    LOG_ENABLED.with(|flag| flag.set(enabled));
    log(&format!(
        "[wasm_voxelizer] logging {}",
        if enabled { "on" } else { "off" }
    ));
}

fn dense_to_sparse(
    dense: voxelizer::core::VoxelizationOutput,
    dims: [u32; 3],
    brick_dim: u32,
) -> voxelizer::core::SparseVoxelizationOutput {
    use std::collections::BTreeMap;

    let brick_voxels = (brick_dim * brick_dim * brick_dim) as usize;
    let words_per_brick = (brick_voxels + 31) / 32;
    let mut bricks: BTreeMap<(u32, u32, u32), (Vec<u32>, Option<Vec<u32>>, Option<Vec<u32>>)> =
        BTreeMap::new();

    let has_owner = dense.owner_id.is_some();
    let has_color = dense.color_rgba.is_some();
    let owner = dense.owner_id.unwrap_or_default();
    let color = dense.color_rgba.unwrap_or_default();

    let total_voxels = (dims[0] as usize) * (dims[1] as usize) * (dims[2] as usize);
    for linear in 0..total_voxels {
        let word = dense.occupancy[linear >> 5];
        let bit = linear & 31;
        if (word & (1u32 << bit)) == 0 {
            continue;
        }

        let x = (linear % dims[0] as usize) as u32;
        let y = ((linear / dims[0] as usize) % dims[1] as usize) as u32;
        let z = (linear / (dims[0] as usize * dims[1] as usize)) as u32;
        let bx = x / brick_dim;
        let by = y / brick_dim;
        let bz = z / brick_dim;
        let ox = bx * brick_dim;
        let oy = by * brick_dim;
        let oz = bz * brick_dim;
        let local = (x - ox) + brick_dim * ((y - oy) + brick_dim * (z - oz));
        let local = local as usize;

        let entry = bricks
            .entry((ox, oy, oz))
            .or_insert_with(|| {
                (
                    vec![0u32; words_per_brick],
                    if has_owner {
                        Some(vec![u32::MAX; brick_voxels])
                    } else {
                        None
                    },
                    if has_color {
                        Some(vec![0u32; brick_voxels])
                    } else {
                        None
                    },
                )
            });
        let word_index = local >> 5;
        let bit_index = local & 31;
        entry.0[word_index] |= 1u32 << bit_index;
        if let Some(owner_store) = entry.1.as_mut() {
            owner_store[local] = owner[linear];
        }
        if let Some(color_store) = entry.2.as_mut() {
            color_store[local] = color[linear];
        }
    }

    let mut brick_origins = Vec::with_capacity(bricks.len());
    let mut occupancy = Vec::new();
    let mut owner_out = Vec::new();
    let mut color_out = Vec::new();
    for ((ox, oy, oz), (occ, own, col)) in bricks.into_iter() {
        brick_origins.push([ox, oy, oz]);
        occupancy.extend(occ);
        if let Some(mut own) = own {
            owner_out.append(&mut own);
        }
        if let Some(mut col) = col {
            color_out.append(&mut col);
        }
    }

    voxelizer::core::SparseVoxelizationOutput {
        brick_dim,
        brick_origins,
        occupancy,
        owner_id: if has_owner { Some(owner_out) } else { None },
        color_rgba: if has_color { Some(color_out) } else { None },
        debug_flags: [0, 0, 0],
        debug_workgroups: 0,
        debug_tested: 0,
        debug_hits: 0,
        stats: dense.stats,
    }
}

const MAX_FALLBACK_BYTES: u64 = 512 * 1024 * 1024;

fn estimate_dense_bytes(dims: [u32; 3], opts: &VoxelizeOpts) -> u64 {
    let num_voxels = dims[0] as u64 * dims[1] as u64 * dims[2] as u64;
    let occupancy_words = (num_voxels + 31) / 32;
    let mut total = occupancy_words.saturating_mul(4);
    if opts.store_owner {
        total = total.saturating_add(num_voxels.saturating_mul(4));
    }
    if opts.store_color {
        total = total.saturating_add(num_voxels.saturating_mul(4));
    }
    total
}

#[wasm_bindgen]
pub struct WasmVoxelizer {
    inner: Rc<GpuVoxelizer>,
}

#[wasm_bindgen]
impl WasmVoxelizer {
    #[wasm_bindgen]
    pub fn new() -> js_sys::Promise {
        future_to_promise(async {
            log("[wasm_voxelizer] init");
            let voxelizer = GpuVoxelizer::new(GpuVoxelizerConfig::default())
                .await
                .map_err(|err| JsValue::from_str(&err))?;
            let limits = voxelizer.limits_summary();
            log(&format!(
                "[wasm_voxelizer] limits invocations={} storage_buffers={} storage_bytes={} workgroups_dim={}",
                limits.max_invocations_per_workgroup,
                limits.max_storage_buffers_per_shader_stage,
                limits.max_storage_buffer_binding_size,
                limits.max_compute_workgroups_per_dimension
            ));
            Ok(JsValue::from(WasmVoxelizer {
                inner: Rc::new(voxelizer),
            }))
        })
    }


    #[wasm_bindgen]
    pub fn voxelize_triangles(
        &self,
        positions: Float32Array,
        indices: Uint32Array,
        origin: Float32Array,
        voxel_size: f32,
        dims: Uint32Array,
        epsilon: f32,
    ) -> js_sys::Promise {
        let positions = positions.to_vec();
        let indices = indices.to_vec();
        let origin = origin.to_vec();
        let dims = dims.to_vec();
        let inner = self.inner.clone();
        future_to_promise(async move {
            log("[wasm_voxelizer] voxelize_triangles");
            if origin.len() < 3 || dims.len() < 3 {
                return Err(JsValue::from_str("origin/dims must have length 3"));
            }

            let mut triangles = Vec::new();
            for face in indices.chunks(3) {
                if face.len() < 3 {
                    continue;
                }
                let idx0 = face[0] as usize * 3;
                let idx1 = face[1] as usize * 3;
                let idx2 = face[2] as usize * 3;
                if idx2 + 2 >= positions.len() {
                    continue;
                }
                let v0 = glam::Vec3::new(positions[idx0], positions[idx0 + 1], positions[idx0 + 2]);
                let v1 = glam::Vec3::new(positions[idx1], positions[idx1 + 1], positions[idx1 + 2]);
                let v2 = glam::Vec3::new(positions[idx2], positions[idx2 + 1], positions[idx2 + 2]);
                triangles.push([v0, v1, v2]);
            }

            let mesh = MeshInput {
                triangles,
                material_ids: None,
            };
            let grid = VoxelGridSpec {
                origin_world: glam::Vec3::new(origin[0], origin[1], origin[2]),
                voxel_size,
                dims: [dims[0], dims[1], dims[2]],
                world_to_grid: None,
            };
            let opts = VoxelizeOpts {
                epsilon,
                store_owner: true,
                store_color: true,
            };

            let output = inner
                .voxelize_surface_sparse(&mesh, &grid, &opts)
                .await
                .map_err(|e| JsValue::from_str(&e))?;

            let mut used_fallback = false;
            let output = if output.occupancy.iter().all(|word| *word == 0) && !mesh.triangles.is_empty() {
                let estimated = estimate_dense_bytes(grid.dims, &opts);
                if estimated > MAX_FALLBACK_BYTES {
                    return Err(JsValue::from_str(&format!(
                        "CPU fallback disabled: estimated dense buffers {} MB exceed limit {} MB",
                        estimated / (1024 * 1024),
                        MAX_FALLBACK_BYTES / (1024 * 1024)
                    )));
                }
                let tile = TileSpec::new([inner.brick_dim(); 3], grid.dims)
                    .map_err(|e| JsValue::from_str(&e))?;
                let dense = voxelize_surface_cpu(&mesh, &grid, &tile, &opts);
                used_fallback = true;
                dense_to_sparse(dense, grid.dims, inner.brick_dim())
            } else {
                output
            };

            let object = Object::new();
            let occupancy = Uint32Array::from(output.occupancy.as_slice());
            Reflect::set(&object, &JsValue::from_str("occupancy"), &occupancy).ok();
            Reflect::set(
                &object,
                &JsValue::from_str("fallback_used"),
                &JsValue::from(used_fallback),
            )
            .ok();
            Reflect::set(
                &object,
                &JsValue::from_str("debug_tested"),
                &JsValue::from(output.debug_tested),
            )
            .ok();
            Reflect::set(
                &object,
                &JsValue::from_str("debug_hits"),
                &JsValue::from(output.debug_hits),
            )
            .ok();
            Reflect::set(
                &object,
                &JsValue::from_str("debug_workgroups"),
                &JsValue::from(output.debug_workgroups),
            )
            .ok();
            let debug_flags = Uint32Array::from(output.debug_flags.as_slice());
            Reflect::set(
                &object,
                &JsValue::from_str("debug_flags"),
                &debug_flags,
            )
            .ok();
            Reflect::set(&object, &JsValue::from_str("brick_dim"), &JsValue::from(output.brick_dim))
                .ok();
            let mut flat = Vec::with_capacity(output.brick_origins.len() * 3);
            for origin in &output.brick_origins {
                flat.push(origin[0]);
                flat.push(origin[1]);
                flat.push(origin[2]);
            }
            let brick_origins = Uint32Array::from(flat.as_slice());
            Reflect::set(
                &object,
                &JsValue::from_str("brick_origins"),
                &brick_origins,
            )
            .ok();
            if let Some(owner) = output.owner_id {
                let owner_arr = Uint32Array::from(owner.as_slice());
                Reflect::set(&object, &JsValue::from_str("owner_id"), &owner_arr).ok();
            }
            if let Some(color) = output.color_rgba {
                let color_arr = Uint32Array::from(color.as_slice());
                Reflect::set(&object, &JsValue::from_str("color_rgba"), &color_arr).ok();
            }
            Reflect::set(
                &object,
                &JsValue::from_str("triangles"),
                &JsValue::from(output.stats.triangles),
            )
            .ok();

            Ok(JsValue::from(object))
        })
    }

    #[wasm_bindgen]
    pub fn voxelize_triangles_positions(
        &self,
        positions: Float32Array,
        indices: Uint32Array,
        origin: Float32Array,
        voxel_size: f32,
        dims: Uint32Array,
        epsilon: f32,
        max_positions: u32,
    ) -> js_sys::Promise {
        let positions = positions.to_vec();
        let indices = indices.to_vec();
        let origin = origin.to_vec();
        let dims = dims.to_vec();
        let inner = self.inner.clone();
        future_to_promise(async move {
            log("[wasm_voxelizer] voxelize_triangles_positions");
            if origin.len() < 3 || dims.len() < 3 {
                return Err(JsValue::from_str("origin/dims must have length 3"));
            }

            let mut triangles = Vec::new();
            for face in indices.chunks(3) {
                if face.len() < 3 {
                    continue;
                }
                let idx0 = face[0] as usize * 3;
                let idx1 = face[1] as usize * 3;
                let idx2 = face[2] as usize * 3;
                if idx2 + 2 >= positions.len() {
                    continue;
                }
                let v0 = glam::Vec3::new(positions[idx0], positions[idx0 + 1], positions[idx0 + 2]);
                let v1 = glam::Vec3::new(positions[idx1], positions[idx1 + 1], positions[idx1 + 2]);
                let v2 = glam::Vec3::new(positions[idx2], positions[idx2 + 1], positions[idx2 + 2]);
                triangles.push([v0, v1, v2]);
            }

            let mesh = MeshInput {
                triangles,
                material_ids: None,
            };
            let grid = VoxelGridSpec {
                origin_world: glam::Vec3::new(origin[0], origin[1], origin[2]),
                voxel_size,
                dims: [dims[0], dims[1], dims[2]],
                world_to_grid: None,
            };
            let opts = VoxelizeOpts {
                epsilon,
                store_owner: true,
                store_color: true,
            };

            let mut used_fallback = false;
            let mut output = inner
                .voxelize_surface_sparse(&mesh, &grid, &opts)
                .await
                .map_err(|e| JsValue::from_str(&e))?;

            if output.occupancy.iter().all(|word| *word == 0) && !mesh.triangles.is_empty() {
                let estimated = estimate_dense_bytes(grid.dims, &opts);
                if estimated > MAX_FALLBACK_BYTES {
                    return Err(JsValue::from_str(&format!(
                        "CPU fallback disabled: estimated dense buffers {} MB exceed limit {} MB",
                        estimated / (1024 * 1024),
                        MAX_FALLBACK_BYTES / (1024 * 1024)
                    )));
                }
                let tile = TileSpec::new([inner.brick_dim(); 3], grid.dims)
                    .map_err(|e| JsValue::from_str(&e))?;
                let dense = voxelize_surface_cpu(&mesh, &grid, &tile, &opts);
                used_fallback = true;
                output = dense_to_sparse(dense, grid.dims, inner.brick_dim());
            }

            let positions = inner
                .compact_sparse_positions(
                    &output.occupancy,
                    &output.brick_origins,
                    output.brick_dim,
                    voxel_size,
                    [origin[0], origin[1], origin[2]],
                    max_positions.max(1),
                )
                .await
                .map_err(|e| JsValue::from_str(&e))?;

            let object = Object::new();
            let positions_arr = Float32Array::from(positions.as_slice());
            Reflect::set(&object, &JsValue::from_str("positions"), &positions_arr).ok();
            Reflect::set(
                &object,
                &JsValue::from_str("brick_dim"),
                &JsValue::from(output.brick_dim),
            )
            .ok();
            Reflect::set(
                &object,
                &JsValue::from_str("brick_count"),
                &JsValue::from(output.brick_origins.len() as u32),
            )
            .ok();
            Reflect::set(
                &object,
                &JsValue::from_str("fallback_used"),
                &JsValue::from(used_fallback),
            )
            .ok();
            Reflect::set(
                &object,
                &JsValue::from_str("debug_workgroups"),
                &JsValue::from(output.debug_workgroups),
            )
            .ok();
            Reflect::set(
                &object,
                &JsValue::from_str("debug_tested"),
                &JsValue::from(output.debug_tested),
            )
            .ok();
            Reflect::set(
                &object,
                &JsValue::from_str("debug_hits"),
                &JsValue::from(output.debug_hits),
            )
            .ok();
            Ok(JsValue::from(object))
        })
    }

    #[wasm_bindgen]
    pub fn voxelize_triangles_sparse_attributes(
        &self,
        positions: Float32Array,
        indices: Uint32Array,
        origin: Float32Array,
        voxel_size: f32,
        dims: Uint32Array,
        epsilon: f32,
        max_entries: u32,
    ) -> js_sys::Promise {
        let positions = positions.to_vec();
        let indices = indices.to_vec();
        let origin = origin.to_vec();
        let dims = dims.to_vec();
        let inner = self.inner.clone();
        future_to_promise(async move {
            log("[wasm_voxelizer] voxelize_triangles_sparse_attributes");
            if origin.len() < 3 || dims.len() < 3 {
                return Err(JsValue::from_str("origin/dims must have length 3"));
            }

            let mut triangles = Vec::new();
            for face in indices.chunks(3) {
                if face.len() < 3 {
                    continue;
                }
                let idx0 = face[0] as usize * 3;
                let idx1 = face[1] as usize * 3;
                let idx2 = face[2] as usize * 3;
                if idx2 + 2 >= positions.len() {
                    continue;
                }
                let v0 = glam::Vec3::new(positions[idx0], positions[idx0 + 1], positions[idx0 + 2]);
                let v1 = glam::Vec3::new(positions[idx1], positions[idx1 + 1], positions[idx1 + 2]);
                let v2 = glam::Vec3::new(positions[idx2], positions[idx2 + 1], positions[idx2 + 2]);
                triangles.push([v0, v1, v2]);
            }

            let mesh = MeshInput {
                triangles,
                material_ids: None,
            };
            let grid = VoxelGridSpec {
                origin_world: glam::Vec3::new(origin[0], origin[1], origin[2]),
                voxel_size,
                dims: [dims[0], dims[1], dims[2]],
                world_to_grid: None,
            };
            let opts = VoxelizeOpts {
                epsilon,
                store_owner: true,
                store_color: true,
            };

            let mut used_fallback = false;
            let mut output = inner
                .voxelize_surface_sparse(&mesh, &grid, &opts)
                .await
                .map_err(|e| JsValue::from_str(&e))?;

            if output.occupancy.iter().all(|word| *word == 0) && !mesh.triangles.is_empty() {
                let estimated = estimate_dense_bytes(grid.dims, &opts);
                if estimated > MAX_FALLBACK_BYTES {
                    return Err(JsValue::from_str(&format!(
                        "CPU fallback disabled: estimated dense buffers {} MB exceed limit {} MB",
                        estimated / (1024 * 1024),
                        MAX_FALLBACK_BYTES / (1024 * 1024)
                    )));
                }
                let tile = TileSpec::new([inner.brick_dim(); 3], grid.dims)
                    .map_err(|e| JsValue::from_str(&e))?;
                let dense = voxelize_surface_cpu(&mesh, &grid, &tile, &opts);
                used_fallback = true;
                output = dense_to_sparse(dense, grid.dims, inner.brick_dim());
            }

            let owner = output
                .owner_id
                .as_ref()
                .ok_or_else(|| JsValue::from_str("owner_id missing from voxelizer output"))?;
            let color = output
                .color_rgba
                .as_ref()
                .ok_or_else(|| JsValue::from_str("color_rgba missing from voxelizer output"))?;
            let max_entries = max_entries.max(1);
            let (indices, owners, colors) = inner
                .compact_sparse_attributes(
                    &output.occupancy,
                    owner,
                    color,
                    &output.brick_origins,
                    output.brick_dim,
                    grid.dims,
                    max_entries,
                )
                .await
                .map_err(|e| JsValue::from_str(&e))?;

            let object = Object::new();
            let indices_arr = Uint32Array::from(indices.as_slice());
            let owners_arr = Uint32Array::from(owners.as_slice());
            let colors_arr = Uint32Array::from(colors.as_slice());
            Reflect::set(&object, &JsValue::from_str("indices"), &indices_arr).ok();
            Reflect::set(&object, &JsValue::from_str("owner_id"), &owners_arr).ok();
            Reflect::set(&object, &JsValue::from_str("color_rgba"), &colors_arr).ok();
            Reflect::set(
                &object,
                &JsValue::from_str("count"),
                &JsValue::from(indices.len() as u32),
            )
            .ok();
            Reflect::set(
                &object,
                &JsValue::from_str("fallback_used"),
                &JsValue::from(used_fallback),
            )
            .ok();
            Ok(JsValue::from(object))
        })
    }

    #[wasm_bindgen]
    pub fn voxelize_triangles_positions_chunked(
        &self,
        positions: Float32Array,
        indices: Uint32Array,
        origin: Float32Array,
        voxel_size: f32,
        dims: Uint32Array,
        epsilon: f32,
        chunk_size: u32,
        max_positions: u32,
    ) -> js_sys::Promise {
        let positions = positions.to_vec();
        let indices = indices.to_vec();
        let origin = origin.to_vec();
        let dims = dims.to_vec();
        let inner = self.inner.clone();
        future_to_promise(async move {
            log("[wasm_voxelizer] voxelize_triangles_positions_chunked");
            if origin.len() < 3 || dims.len() < 3 {
                return Err(JsValue::from_str("origin/dims must have length 3"));
            }
            let mut triangles = Vec::new();
            for face in indices.chunks(3) {
                if face.len() < 3 {
                    continue;
                }
                let idx0 = face[0] as usize * 3;
                let idx1 = face[1] as usize * 3;
                let idx2 = face[2] as usize * 3;
                if idx2 + 2 >= positions.len() {
                    continue;
                }
                let v0 = glam::Vec3::new(positions[idx0], positions[idx0 + 1], positions[idx0 + 2]);
                let v1 = glam::Vec3::new(positions[idx1], positions[idx1 + 1], positions[idx1 + 2]);
                let v2 = glam::Vec3::new(positions[idx2], positions[idx2 + 1], positions[idx2 + 2]);
                triangles.push([v0, v1, v2]);
            }
            let mesh = MeshInput {
                triangles,
                material_ids: None,
            };
            let grid = VoxelGridSpec {
                origin_world: glam::Vec3::new(origin[0], origin[1], origin[2]),
                voxel_size,
                dims: [dims[0], dims[1], dims[2]],
                world_to_grid: None,
            };
            let opts = VoxelizeOpts {
                epsilon,
                store_owner: true,
                store_color: true,
            };
            let chunk_size = chunk_size.max(1) as usize;
            let mut chunks = inner
                .voxelize_surface_sparse_chunked(&mesh, &grid, &opts, chunk_size)
                .await
                .map_err(|e| JsValue::from_str(&e))?;
            if chunks.is_empty() {
                return Ok(JsValue::from(js_sys::Array::new()));
            }
            if !chunks
                .iter()
                .any(|chunk| chunk.occupancy.iter().any(|word| *word != 0))
            {
                let estimated = estimate_dense_bytes(grid.dims, &opts);
                if estimated > MAX_FALLBACK_BYTES {
                    return Err(JsValue::from_str(&format!(
                        "CPU fallback disabled: estimated dense buffers {} MB exceed limit {} MB",
                        estimated / (1024 * 1024),
                        MAX_FALLBACK_BYTES / (1024 * 1024)
                    )));
                }
                log("[wasm_voxelizer] gpu compact fallback to CPU sparse");
                let tile = TileSpec::new([inner.brick_dim(); 3], grid.dims)
                    .map_err(|e| JsValue::from_str(&e))?;
                let dense = voxelize_surface_cpu(&mesh, &grid, &tile, &opts);
                chunks = vec![dense_to_sparse(dense, grid.dims, inner.brick_dim())];
            }
            let per_chunk = (max_positions / chunks.len() as u32).max(1);
            log(&format!(
                "[wasm_voxelizer] gpu_compact per_chunk={} chunks={}",
                per_chunk,
                chunks.len()
            ));
            let array = js_sys::Array::new();
            let mut logged = 0usize;
            for chunk in chunks {
                if logged < 3 {
                    log(&format!(
                        "[wasm_voxelizer] gpu_compact chunk bricks={} occupancy_words={}",
                        chunk.brick_origins.len(),
                        chunk.occupancy.len()
                    ));
                    logged += 1;
                }
                let positions = inner
                    .compact_sparse_positions(
                        &chunk.occupancy,
                        &chunk.brick_origins,
                        chunk.brick_dim,
                        voxel_size,
                        [origin[0], origin[1], origin[2]],
                        per_chunk,
                    )
                    .await
                    .map_err(|e| JsValue::from_str(&e))?;
                let object = Object::new();
                let positions_arr = Float32Array::from(positions.as_slice());
                Reflect::set(&object, &JsValue::from_str("positions"), &positions_arr).ok();
                Reflect::set(
                    &object,
                    &JsValue::from_str("count"),
                    &JsValue::from((positions.len() / 3) as u32),
                )
                .ok();
                Reflect::set(
                    &object,
                    &JsValue::from_str("brick_dim"),
                    &JsValue::from(chunk.brick_dim),
                )
                .ok();
                Reflect::set(
                    &object,
                    &JsValue::from_str("brick_count"),
                    &JsValue::from(chunk.brick_origins.len() as u32),
                )
                .ok();
                array.push(&object);
            }
            Ok(JsValue::from(array))
        })
    }

    #[wasm_bindgen]
    #[wasm_bindgen]
    pub fn voxelize_triangles_chunked(
        &self,
        positions: Float32Array,
        indices: Uint32Array,
        origin: Float32Array,
        voxel_size: f32,
        dims: Uint32Array,
        epsilon: f32,
        chunk_size: u32,
        compact: bool,
    ) -> js_sys::Promise {
        let positions = positions.to_vec();
        let indices = indices.to_vec();
        let origin = origin.to_vec();
        let dims = dims.to_vec();

        let inner = self.inner.clone();
        future_to_promise(async move {
            log("[wasm_voxelizer] voxelize_triangles_chunked");
            if origin.len() < 3 || dims.len() < 3 {
                return Err(JsValue::from_str("origin/dims must have length 3"));
            }

            let mut triangles = Vec::new();
            for face in indices.chunks(3) {
                if face.len() < 3 {
                    continue;
                }
                let idx0 = face[0] as usize * 3;
                let idx1 = face[1] as usize * 3;
                let idx2 = face[2] as usize * 3;
                if idx2 + 2 >= positions.len() {
                    continue;
                }
                let v0 = glam::Vec3::new(positions[idx0], positions[idx0 + 1], positions[idx0 + 2]);
                let v1 = glam::Vec3::new(positions[idx1], positions[idx1 + 1], positions[idx1 + 2]);
                let v2 = glam::Vec3::new(positions[idx2], positions[idx2 + 1], positions[idx2 + 2]);
                triangles.push([v0, v1, v2]);
            }

            let mesh = MeshInput {
                triangles,
                material_ids: None,
            };
            let grid = VoxelGridSpec {
                origin_world: glam::Vec3::new(origin[0], origin[1], origin[2]),
                voxel_size,
                dims: [dims[0], dims[1], dims[2]],
                world_to_grid: None,
            };

            let opts = VoxelizeOpts {
                epsilon,
                store_owner: true,
                store_color: true,
            };

            if chunk_size == 0 {
                log("[wasm_voxelizer] chunk_size=auto");
            }
            let chunk_size = chunk_size as usize;
            let chunks = inner
                .voxelize_surface_sparse_chunked(&mesh, &grid, &opts, chunk_size)
                .await
                .map_err(|e| JsValue::from_str(&e))?;
            log(&format!(
                "[wasm_voxelizer] chunked outputs: {} chunks",
                chunks.len()
            ));

            let gpu_debug_tested: u32 = chunks.iter().map(|c| c.debug_tested).sum();
            let gpu_debug_hits: u32 = chunks.iter().map(|c| c.debug_hits).sum();
            let gpu_debug_workgroups: u32 = chunks.iter().map(|c| c.debug_workgroups).sum();
            let mut gpu_debug_flags = [0u32; 3];
            for chunk in &chunks {
                for (idx, value) in chunk.debug_flags.iter().enumerate() {
                    gpu_debug_flags[idx] |= *value;
                }
            }
            log(&format!(
                "[wasm_voxelizer] gpu_debug workgroups={} tested={} hits={} flags={:?}",
                gpu_debug_workgroups, gpu_debug_tested, gpu_debug_hits, gpu_debug_flags
            ));

            let mut cpu_fallback_used = false;
            let mut chunks = if chunks.iter().any(|chunk| chunk.occupancy.iter().any(|w| *w != 0)) {
                chunks
            } else if !mesh.triangles.is_empty() {
                let estimated = estimate_dense_bytes(grid.dims, &opts);
                if estimated > MAX_FALLBACK_BYTES {
                    return Err(JsValue::from_str(&format!(
                        "CPU fallback disabled: estimated dense buffers {} MB exceed limit {} MB",
                        estimated / (1024 * 1024),
                        MAX_FALLBACK_BYTES / (1024 * 1024)
                    )));
                }
                log("[wasm_voxelizer] chunked fallback to CPU sparse");
                let tile = TileSpec::new([inner.brick_dim(); 3], grid.dims)
                    .map_err(|e| JsValue::from_str(&e))?;
                let dense = voxelize_surface_cpu(&mesh, &grid, &tile, &opts);
                cpu_fallback_used = true;
                vec![dense_to_sparse(dense, grid.dims, inner.brick_dim())]
            } else {
                chunks
            };

            let array = js_sys::Array::new();
            for chunk in chunks.drain(..) {
                log(&format!(
                    "[wasm_voxelizer] chunk bricks={}",
                    chunk.brick_origins.len()
                ));
                let chunk = if compact { compact_sparse(chunk) } else { chunk };
                let object = Object::new();
                let occupancy = Uint32Array::from(chunk.occupancy.as_slice());
                Reflect::set(&object, &JsValue::from_str("occupancy"), &occupancy).ok();
                Reflect::set(&object, &JsValue::from_str("brick_dim"), &JsValue::from(chunk.brick_dim))
                    .ok();
                Reflect::set(
                    &object,
                    &JsValue::from_str("debug_tested"),
                    &JsValue::from(if chunk.debug_tested == 0 { gpu_debug_tested } else { chunk.debug_tested }),
                )
                .ok();
                Reflect::set(
                    &object,
                    &JsValue::from_str("debug_hits"),
                    &JsValue::from(if chunk.debug_hits == 0 { gpu_debug_hits } else { chunk.debug_hits }),
                )
                .ok();
                Reflect::set(
                    &object,
                    &JsValue::from_str("debug_workgroups"),
                    &JsValue::from(if chunk.debug_workgroups == 0 {
                        gpu_debug_workgroups
                    } else {
                        chunk.debug_workgroups
                    }),
                )
                .ok();
                let debug_flags = if chunk.debug_flags.iter().all(|v| *v == 0) {
                    Uint32Array::from(gpu_debug_flags.as_slice())
                } else {
                    Uint32Array::from(chunk.debug_flags.as_slice())
                };
                Reflect::set(
                    &object,
                    &JsValue::from_str("debug_flags"),
                    &debug_flags,
                )
                .ok();
                Reflect::set(
                    &object,
                    &JsValue::from_str("fallback_used"),
                    &JsValue::from(cpu_fallback_used),
                )
                .ok();
                let mut flat = Vec::with_capacity(chunk.brick_origins.len() * 3);
                for origin in &chunk.brick_origins {
                    flat.push(origin[0]);
                    flat.push(origin[1]);
                    flat.push(origin[2]);
                }
                let brick_origins = Uint32Array::from(flat.as_slice());
                Reflect::set(
                    &object,
                    &JsValue::from_str("brick_origins"),
                    &brick_origins,
                )
                .ok();
                array.push(&object);
            }

            Ok(JsValue::from(array))
        })
    }
}

fn compact_sparse(
    input: voxelizer::core::SparseVoxelizationOutput,
) -> voxelizer::core::SparseVoxelizationOutput {
    let brick_count = input.brick_origins.len();
    let brick_voxels = (input.brick_dim * input.brick_dim * input.brick_dim) as usize;
    let words_per_brick = (brick_voxels + 31) / 32;

    let mut brick_origins = Vec::new();
    let mut occupancy = Vec::new();
    let mut owner_out = Vec::new();
    let mut color_out = Vec::new();

    let owner = input.owner_id.unwrap_or_default();
    let color = input.color_rgba.unwrap_or_default();
    let has_owner = !owner.is_empty();
    let has_color = !color.is_empty();

    for brick_index in 0..brick_count {
        let start = brick_index * words_per_brick;
        let end = start + words_per_brick;
        let has_any = input.occupancy[start..end].iter().any(|word| *word != 0);
        if !has_any {
            continue;
        }
        brick_origins.push(input.brick_origins[brick_index]);
        occupancy.extend_from_slice(&input.occupancy[start..end]);
        if has_owner {
            let base = brick_index * brick_voxels;
            owner_out.extend_from_slice(&owner[base..base + brick_voxels]);
        }
        if has_color {
            let base = brick_index * brick_voxels;
            color_out.extend_from_slice(&color[base..base + brick_voxels]);
        }
    }

    voxelizer::core::SparseVoxelizationOutput {
        brick_dim: input.brick_dim,
        brick_origins,
        occupancy,
        owner_id: if has_owner { Some(owner_out) } else { None },
        color_rgba: if has_color { Some(color_out) } else { None },
        debug_flags: input.debug_flags,
        debug_workgroups: input.debug_workgroups,
        debug_tested: input.debug_tested,
        debug_hits: input.debug_hits,
        stats: input.stats,
    }
}
