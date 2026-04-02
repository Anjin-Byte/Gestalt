// Build Wireframe Indices
//
// Converts quad triangle indices [v0,v1,v2, v0,v2,v3] into edge line indices
// [v0,v1, v1,v2, v2,v3, v3,v0] for LineList topology rendering.
// Also writes wireframe indirect draw args.
//
// Reads source triangle indices from mesh_offset_table offsets (variable pool).
// Wire output still uses fixed per-slot allocation (MAX_WIRE_INDICES_PER_CHUNK).
//
// Dispatch: (slot_count, 1, 1), @workgroup_size(64, 1, 1).

const MAX_SLOTS: u32 = 4096u;
const MAX_WIRE_INDICES_PER_CHUNK: u32 = 32768u;
const INDIRECT_STRIDE: u32 = 5u;

// ─── Bindings ───────────────────────────────────────────────────────────

@group(0) @binding(0) var<storage, read>       index_pool:        array<u32>;
@group(0) @binding(1) var<storage, read>       mesh_offset_table: array<u32>;
@group(0) @binding(2) var<storage, read_write> wire_index_pool:   array<u32>;
@group(0) @binding(3) var<storage, read_write> wire_indirect:     array<u32>;

// ─── Entry point ────────────────────────────────────────────────────────

@compute @workgroup_size(64, 1, 1)
fn main(
    @builtin(workgroup_id) wg_id: vec3u,
    @builtin(local_invocation_id) local_id: vec3u,
) {
    let slot = wg_id.x;
    if slot >= MAX_SLOTS { return; }

    let tid = local_id.x;
    let ot_base = slot * 5u;
    let vert_offset = mesh_offset_table[ot_base];
    let idx_offset  = mesh_offset_table[ot_base + 2u];
    let idx_count   = mesh_offset_table[ot_base + 3u];
    let quad_count = idx_count / 6u;

    let src_base = idx_offset;
    let dst_base = slot * MAX_WIRE_INDICES_PER_CHUNK;  // wire output still fixed

    var q = tid;
    while q < quad_count {
        let si = src_base + q * 6u;
        let v0 = index_pool[si];
        let v1 = index_pool[si + 1u];
        let v2 = index_pool[si + 2u];
        let v3 = index_pool[si + 5u];

        let di = dst_base + q * 8u;
        wire_index_pool[di]      = v0;
        wire_index_pool[di + 1u] = v1;
        wire_index_pool[di + 2u] = v1;
        wire_index_pool[di + 3u] = v2;
        wire_index_pool[di + 4u] = v2;
        wire_index_pool[di + 5u] = v3;
        wire_index_pool[di + 6u] = v3;
        wire_index_pool[di + 7u] = v0;

        q += 64u;
    }

    workgroupBarrier();

    if tid == 0u {
        let wire_count = quad_count * 8u;
        let ind_base = slot * INDIRECT_STRIDE;
        wire_indirect[ind_base]      = wire_count;
        wire_indirect[ind_base + 1u] = select(0u, 1u, wire_count > 0u);
        wire_indirect[ind_base + 2u] = slot * MAX_WIRE_INDICES_PER_CHUNK;
        wire_indirect[ind_base + 3u] = vert_offset;  // base_vertex from variable pool
        wire_indirect[ind_base + 4u] = 0u;
    }
}
