# WASM API and Worker Protocol

Date: February 22, 2026
Status: Authoritative

Source: Refined from `archive/voxelizer-greedy-integration-spec.md` §§3.1–3.5.
Internal execution flow updated for Architecture B (GPU outputs `CompactVoxel[]`
directly; no intermediate `SparseVoxelizationOutput` exposed to the CPU ingestion
layer).

---

## Architectural Principle

The integrated voxelizer lives **inside** `crates/wasm_greedy_mesher`. The JS
caller invokes one async function; all GPU dispatch, compact output processing,
and chunk ingestion happen within that single WASM call. No intermediate format
crosses the JS boundary between GPU and chunk manager.

This eliminates:
- Cross-worker message overhead and Transferable protocols for voxel data
- Any intermediate binary format (`VoxelizerChunkDeltaBatch` from Architecture A)
- Session ordering state machines previously required to coordinate two workers
- The risk of voxelizer and chunk manager modules drifting out of sync

The legacy `crates/wasm_voxelizer` module is unchanged and continues to serve its
existing debug/preview use case.

---

## `init_voxelizer` — GPU Device Initialization

```rust
/// Initialize the embedded GPU voxelizer for this chunk manager.
/// Must be called once before voxelize_and_apply.
/// Obtains a wgpu Device/Queue via the default WebGPU adapter.
///
/// Returns Promise<bool>: true on success, false if WebGPU unavailable.
#[wasm_bindgen]
pub fn init_voxelizer(&mut self) -> js_sys::Promise
```

**Lifecycle:** Call once after constructing `WasmChunkManager`, before any
`voxelize_and_apply` call. Subsequent calls are no-ops if already initialized.

**Implementation pattern (mirrors `wasm_voxelizer`):**
Clone `Rc<RefCell<Option<GpuVoxelizer>>>` before the async block. After
`GpuVoxelizer::new(default).await` resolves, assign the result into the
`RefCell`. The `Rc` clone bridges the `&mut self` borrow across the `await` point.

---

## `voxelize_and_apply` — Single-Call Integration

```rust
/// GPU-voxelize a triangle mesh and apply the resulting voxels directly
/// into this chunk manager's storage.
///
/// All GPU dispatch, compact output processing, and chunk ingestion happen
/// inside this call. Dirty marking is performed after all GPU batches complete.
///
/// Parameters:
///   positions      — flat f32 vertex positions [x0,y0,z0, ...]
///   indices        — flat u32 triangle index array (length must be 3*n_triangles)
///   material_table — u16 MaterialId per triangle; material_table[i] = MaterialId
///                    for triangle i (1-based; build with buildMaterialTable())
///   origin_x/y/z   — world-space origin of the voxelizer grid (must satisfy VOX-ALIGN)
///   dim_x/y/z      — grid dimensions in voxels
///   epsilon        — triangle–voxel overlap tolerance (default 1e-4)
///
/// Returns Promise<u32> — count of voxels written into the chunk manager.
/// Rejects if:
///   - init_voxelizer() was not called or failed
///   - origin is not VOX-ALIGN aligned (see spec/coordinate-frames.md)
///   - material_table.length != indices.length / 3
#[wasm_bindgen]
pub fn voxelize_and_apply(
    &mut self,
    positions:      js_sys::Float32Array,
    indices:        js_sys::Uint32Array,
    material_table: js_sys::Uint16Array,
    origin_x: f32, origin_y: f32, origin_z: f32,
    dim_x: u32, dim_y: u32, dim_z: u32,
    epsilon: f32,
) -> js_sys::Promise
```

---

## Internal Execution Flow

The body of `voxelize_and_apply` follows this sequence entirely in Rust:

```
1. Validate inputs
   a. VOX-ALIGN: for each origin component, abs(ratio - round(ratio)) <= 1e-4
   b. material_table.length == indices.length / 3
   c. init_voxelizer() was called (gpu_voxelizer is Some)

2. Copy typed arrays to owned Vecs
   — WASM memory views cannot be held across await points

3. Pack material_table: Vec<u16> → Vec<u32>
   — two u16 per u32 word (see spec/material-pipeline.md)

4. Read voxel_size = self.inner.borrow().voxel_size()

5. Compute g_origin = [floor(origin_x/vs), floor(origin_y/vs), floor(origin_z/vs)]

6. Clone Rc refs before async block:
   — gpu = self.gpu_voxelizer.clone()
   — manager = self.inner.clone()

7. future_to_promise(async move {
     a. Build MeshInput triangles from positions_vec + indices_vec.chunks(3)
     b. MeshInput { triangles, material_ids: None }
        — attribution is fully handled by the GPU material_table lookup
     c. VoxelGridSpec {
            origin_world: Vec3::new(origin_x, origin_y, origin_z),
            voxel_size,
            dims: [dim_x, dim_y, dim_z],
            world_to_grid: None,
        }
     d. VoxelizeOpts { epsilon, store_owner: true, store_color: false }
     e. compact_voxels: Vec<CompactVoxel> =
            gpu.compact_surface_sparse(
                &mesh, &grid, &opts, &mat_packed, g_origin
            ).await?
        — GPU compact pass returns occupied voxels with global coords + materials
     f. count = compact_to_chunk_writes(
            &compact_voxels,
            &mut manager.borrow_mut()
        )
        — groups by chunk, writes voxels, marks dirty (see design/cpu-ingestion.md)
     g. Ok(JsValue::from(count as u32))
   })
```

**Why `Rc<RefCell<>>` and not `&mut self` in the async block:**
`&mut self` cannot be held across an `await` point in `wasm_bindgen`'s async
model (the future must be `'static`). Wrapping `inner` and `gpu_voxelizer` in
`Rc<RefCell<>>` allows cloning the `Rc` before the async block and acquiring
`RefCell` borrows only within synchronous segments. Since WASM is single-threaded,
the borrow is never contended at runtime; `RefCell` panics cannot occur unless
the borrow checker is bypassed by recursive JS callbacks during an await, which
cannot happen for GPU readback awaits.

---

## Worker Message Protocol

One worker handles both chunk management and voxelization in the integrated design.

### Request (application → worker)

```typescript
interface VoxelizeAndApplyRequest {
    readonly type:          'cm-voxelize-and-apply';
    readonly sessionId:     number;
    readonly positions:     Float32Array;     // Transferable
    readonly indices:       Uint32Array;      // Transferable
    readonly materialTable: Uint16Array;      // Transferable
    readonly origin:        [number, number, number];
    readonly dims:          [number, number, number];
    readonly epsilon:       number;
}

// Transfer the three geometry arrays to zero-copy them into the worker:
worker.postMessage(req, [
    req.positions.buffer,
    req.indices.buffer,
    req.materialTable.buffer,
]);
```

No voxel data ever crosses a worker boundary in either direction. The worker
receives geometry, performs GPU voxelization and chunk ingestion internally, and
returns only a count.

### Response (worker → application)

```typescript
interface VoxelizeAndApplyResult {
    readonly type:          'cm-voxelize-and-apply-result';
    readonly sessionId:     number;
    readonly voxelsWritten: number;
    readonly error:         string | null;
}
```

Existing chunk-update notifications (`last_swapped_coords`, `last_evicted_coords`)
continue to fire on the next `update()` call after dirty chunks are rebuilt.

---

## Backpressure

The chunk manager worker's event loop is cooperative. While `voxelize_and_apply`
is executing (including between GPU await points), the worker cannot process other
messages. A second `cm-voxelize-and-apply` message will not be processed until
the current one resolves its Promise.

For the expected use case (user places one mesh at a time), inter-request spacing
is at minimum hundreds of milliseconds — far above typical voxelization latency.
No explicit queue depth limit is required.

---

## TypeScript Usage Example

```typescript
import { buildMaterialTable, parseObjFallback }
    from '../wasmObjLoader/helpers';

// 1. Initialize once after creating WasmChunkManager
const ok = await manager.init_voxelizer();
if (!ok) throw new Error('WebGPU unavailable');

// 2. Parse OBJ
const { positions, indices, triangleMaterials, materialGroupNames }
    = parseObjFallback(objText);

// 3. Build material table (tri_idx → MaterialId, 1-based)
const materialTable = buildMaterialTable(triangleMaterials, materialGroupNames);
// materialTable.length === indices.length / 3

// 4. Snap origin to voxel grid (VOX-ALIGN requirement)
const vs = manager.voxel_size();
const snap = (v: number) => Math.round(v / vs) * vs;
const [ox, oy, oz] = [snap(rawOx), snap(rawOy), snap(rawOz)];

// 5. Voxelize and write into chunk manager in one call
const count = await manager.voxelize_and_apply(
    positions, indices, materialTable,
    ox, oy, oz,
    gridDim, gridDim, gridDim,
    1e-4
);

// 6. Normal frame update — dirty chunks rebuild with material-aware merge
manager.update(camX, camY, camZ);

// 7. Mesh contains per-vertex material_ids ready for atlas lookup
const mesh = manager.get_chunk_mesh(cx, cy, cz);
// mesh.material_ids: Uint16Array — distinct values per usemtl region
```
