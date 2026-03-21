# WasmChunkManager Extension

**Type:** spec
**Status:** proposed
**Date:** 2026-02-22

File modified: `crates/wasm_greedy_mesher/src/lib.rs`
File modified: `crates/wasm_greedy_mesher/Cargo.toml`

---

## `Cargo.toml` Additions

```toml
[dependencies]
# ... existing deps ...
greedy_voxelizer     = { path = "../greedy_voxelizer" }
voxelizer            = { path = "../voxelizer" }
wasm-bindgen-futures = "0.4"
glam                 = "0.27"
wgpu                 = { version = "22", features = ["webgpu"] }
```

---

## `WasmChunkManager` Struct Change

### Before

```rust
#[wasm_bindgen]
pub struct WasmChunkManager {
    inner: ChunkManager,
}
```

### After

```rust
#[wasm_bindgen]
pub struct WasmChunkManager {
    inner:         Rc<RefCell<ChunkManager>>,
    gpu_voxelizer: Rc<RefCell<Option<GpuVoxelizer>>>,
}
```

**Why `Rc<RefCell<>>` on both fields:**

`voxelize_and_apply` is an async function (returns `js_sys::Promise`). The future
must be `'static` for `wasm_bindgen_futures::future_to_promise`. This means
`&mut self` cannot be used inside the async block — the reference cannot outlive
the call frame.

The solution: clone `Rc`s before the async block (cheap — just increments refcount),
then acquire `RefCell` borrows only within synchronous segments between `await`
points. Since WASM is single-threaded, the borrow is never contended at runtime.

**All existing synchronous methods** that previously did `self.inner.method()` now
do `self.inner.borrow_mut().method()` or `self.inner.borrow().method()`. Their
external signatures are unchanged.

### Constructor Updates

All three constructors (`new`, `with_config`, `with_budget`) wrap the
`ChunkManager` and initialize `gpu_voxelizer`:

```rust
pub fn new() -> Self {
    Self {
        inner:         Rc::new(RefCell::new(ChunkManager::default())),
        gpu_voxelizer: Rc::new(RefCell::new(None)),
    }
}
```

---

## `init_voxelizer` Method

```rust
/// Initialize the embedded GPU voxelizer.
/// Must be called once before voxelize_and_apply.
/// Returns Promise<bool>: true on success, false if WebGPU unavailable.
#[wasm_bindgen]
pub fn init_voxelizer(&self) -> js_sys::Promise {
    let voxelizer_cell = self.gpu_voxelizer.clone();

    future_to_promise(async move {
        match GpuVoxelizer::new(GpuVoxelizerConfig::default()).await {
            Ok(v) => {
                *voxelizer_cell.borrow_mut() = Some(v);
                Ok(JsValue::from(true))
            }
            Err(e) => {
                web_sys::console::warn_1(
                    &JsValue::from_str(&format!("init_voxelizer failed: {}", e))
                );
                Ok(JsValue::from(false))
            }
        }
    })
}
```

Note: takes `&self` (not `&mut self`) because the mutation goes through `RefCell`.

---

## `voxelize_and_apply` Method

```rust
/// GPU-voxelize a triangle mesh and write results into this chunk manager.
/// All GPU dispatch, compact output processing, and chunk ingestion happen
/// within this call. Returns Promise<u32> — count of voxels written.
#[wasm_bindgen]
pub fn voxelize_and_apply(
    &self,
    positions:      js_sys::Float32Array,
    indices:        js_sys::Uint32Array,
    material_table: js_sys::Uint16Array,
    origin_x: f32, origin_y: f32, origin_z: f32,
    dim_x: u32, dim_y: u32, dim_z: u32,
    epsilon: f32,
) -> js_sys::Promise {

    // 1. Copy typed arrays to owned Vecs before async block.
    //    WASM memory views cannot be held across await points.
    let positions_vec: Vec<f32> = positions.to_vec();
    let indices_vec:   Vec<u32> = indices.to_vec();
    let mat_vec:       Vec<u16> = material_table.to_vec();

    // 2. Read voxel_size synchronously before entering async block.
    let voxel_size = self.inner.borrow().voxel_size();

    // 3. Validation before GPU work.
    //    a. VOX-ALIGN
    for (i, &o) in [origin_x, origin_y, origin_z].iter().enumerate() {
        let ratio = o / voxel_size;
        if (ratio - ratio.round()).abs() > 1e-4 {
            return future_to_promise(async move {
                Err(JsValue::from_str(&format!(
                    "origin_world[{}]={} not aligned to voxel_size={}", i, o, voxel_size
                )))
            });
        }
    }
    //    b. material_table length
    let n_triangles = indices_vec.len() / 3;
    if mat_vec.len() != n_triangles {
        return future_to_promise(async move {
            Err(JsValue::from_str(&format!(
                "material_table.len()={} != n_triangles={}",
                mat_vec.len(), n_triangles
            )))
        });
    }

    // 4. Pack material_table: two u16 per u32 word.
    let mat_packed: Vec<u32> = mat_vec.chunks(2).map(|pair| {
        (pair[0] as u32) | ((pair.get(1).copied().unwrap_or(0) as u32) << 16)
    }).collect();

    // 5. Compute g_origin.
    let g_origin = [
        (origin_x / voxel_size).floor() as i32,
        (origin_y / voxel_size).floor() as i32,
        (origin_z / voxel_size).floor() as i32,
    ];

    // 6. Clone Rc refs before async block.
    let gpu_cell     = self.gpu_voxelizer.clone();
    let manager_cell = self.inner.clone();

    future_to_promise(async move {
        // 7. Borrow GPU voxelizer — fail early if not initialized.
        let gpu_ref = gpu_cell.borrow();
        let gpu = gpu_ref.as_ref().ok_or_else(|| {
            JsValue::from_str("voxelizer not initialized; call init_voxelizer() first")
        })?;

        // 8. Build MeshInput from positions + indices.
        let triangles: Vec<[glam::Vec3; 3]> = indices_vec
            .chunks(3)
            .map(|tri| {
                let v = |i: u32| {
                    let b = (i as usize) * 3;
                    glam::Vec3::new(positions_vec[b], positions_vec[b+1], positions_vec[b+2])
                };
                [v(tri[0]), v(tri[1]), v(tri[2])]
            })
            .collect();
        let mesh = voxelizer::core::MeshInput { triangles, material_ids: None };

        // 9. Build VoxelGridSpec.
        let grid = voxelizer::core::VoxelGridSpec {
            origin_world: glam::Vec3::new(origin_x, origin_y, origin_z),
            voxel_size,
            dims: [dim_x, dim_y, dim_z],
            world_to_grid: None,
        };

        // 10. Build VoxelizeOpts.
        let opts = voxelizer::core::VoxelizeOpts {
            epsilon,
            store_owner: true,
            store_color: false,   // debug color removed from compact output
        };

        // 11. GPU compact pass — returns Vec<CompactVoxel>.
        //     (GpuVoxelizer::compact_surface_sparse is the new method on the voxelizer)
        let compact_voxels = gpu
            .compact_surface_sparse(&mesh, &grid, &opts, &mat_packed, g_origin)
            .await
            .map_err(|e| JsValue::from_str(&e))?;

        // 12. CPU ingestion — groups by chunk, writes voxels, marks dirty.
        //     Borrow acquired here, released at end of this block.
        drop(gpu_ref);   // release GPU borrow before manager borrow (avoid any possible ordering issue)
        let mut manager = manager_cell.borrow_mut();
        let count = greedy_voxelizer::compact_to_chunk_writes(
            &compact_voxels,
            &mut *manager,
        );

        Ok(JsValue::from(count as u32))
    })
}
```

---

## Updating Existing Methods

Every existing method that accesses `inner` must be updated:

```rust
// Before
pub fn set_voxel_at(&mut self, x: f32, y: f32, z: f32, material: u16) {
    self.inner.set_voxel_at(x, y, z, material);
}

// After
pub fn set_voxel_at(&mut self, x: f32, y: f32, z: f32, material: u16) {
    self.inner.borrow_mut().set_voxel_at(x, y, z, material);
}
```

The pattern is mechanical: `self.inner.x(...)` → `self.inner.borrow_mut().x(...)`
or `self.inner.borrow().x(...)` depending on mutability.

---

## `GpuVoxelizer::compact_surface_sparse`

This is the new method on `GpuVoxelizer` in `crates/voxelizer` that the WASM
bindings call. It wraps the updated `compact_sparse_attributes` from Phase 1:

```rust
impl GpuVoxelizer {
    /// Run the compact voxelization pass and return CompactVoxel output.
    pub async fn compact_surface_sparse(
        &self,
        mesh:           &MeshInput,
        grid:           &VoxelGridSpec,
        opts:           &VoxelizeOpts,
        material_table: &[u32],       // packed u16 pairs
        g_origin:       [i32; 3],
    ) -> Result<Vec<CompactVoxel>, String>
}
```

This method builds the brick CSR, batches over GPU dispatches, and for each
batch calls the updated `compact_sparse_attributes`, accumulating results into
a single `Vec<CompactVoxel>`.
