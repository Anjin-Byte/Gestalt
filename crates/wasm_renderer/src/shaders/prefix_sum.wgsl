// R-1 Pass 2: Prefix Sum — computes per-slot mesh pool offsets from quad counts.
//
// Exclusive prefix sum (Blelloch scan) over mesh_counts[0..MAX_SLOTS].
// Output: mesh_offset_table[slot] = vec4u(vert_offset, vert_count, idx_offset, idx_count)
//         mesh_total = (total_vertices, total_indices)
//
// Dispatch: (1, 1, 1) — single workgroup.
//
// See: docs/Resident Representation/variable-mesh-pool.md

const MAX_SLOTS: u32 = 4096u;
const ELEMS_PER_THREAD: u32 = 16u;  // 4096 slots / 256 threads

@group(0) @binding(0) var<storage, read>       mesh_counts:       array<u32>;
@group(0) @binding(1) var<storage, read_write> mesh_offset_table: array<u32>;
@group(0) @binding(2) var<storage, read_write> mesh_total:        array<u32>;

var<workgroup> shared_data: array<u32, 4096>;

@compute @workgroup_size(256, 1, 1)
fn main(@builtin(local_invocation_id) lid: vec3u) {
    let tid = lid.x;

    // ── Load quad counts into shared memory ──
    for (var i = 0u; i < ELEMS_PER_THREAD; i++) {
        let idx = tid * ELEMS_PER_THREAD + i;
        if idx < MAX_SLOTS {
            shared_data[idx] = mesh_counts[idx];
        }
    }
    workgroupBarrier();

    // ── Blelloch up-sweep (reduce) ──
    // Build partial sums in a binary tree pattern.
    var offset = 1u;
    var n = MAX_SLOTS;
    loop {
        if offset >= n { break; }
        let step = offset * 2u;
        for (var i = 0u; i < ELEMS_PER_THREAD; i++) {
            let idx = tid * ELEMS_PER_THREAD + i;
            if idx < n && ((idx + 1u) % step) == 0u {
                shared_data[idx] += shared_data[idx - offset];
            }
        }
        workgroupBarrier();
        offset *= 2u;
    }

    // ── Store total and clear last element ──
    if tid == 0u {
        let total_quads = shared_data[MAX_SLOTS - 1u];
        mesh_total[0] = total_quads * 4u;  // total vertices
        mesh_total[1] = total_quads * 6u;  // total indices
        shared_data[MAX_SLOTS - 1u] = 0u;
    }
    workgroupBarrier();

    // ── Blelloch down-sweep (distribute) ──
    offset = n / 2u;
    loop {
        if offset == 0u { break; }
        let step = offset * 2u;
        for (var i = 0u; i < ELEMS_PER_THREAD; i++) {
            let idx = tid * ELEMS_PER_THREAD + i;
            if idx < n && ((idx + 1u) % step) == 0u {
                let temp = shared_data[idx - offset];
                shared_data[idx - offset] = shared_data[idx];
                shared_data[idx] += temp;
            }
        }
        workgroupBarrier();
        offset /= 2u;
    }

    // ── Write offset table ──
    // shared_data[i] now contains the exclusive prefix sum (total quads before slot i).
    // Layout: 5 u32 per slot [vert_offset, vert_count, idx_offset, idx_count, write_counter=0]
    for (var i = 0u; i < ELEMS_PER_THREAD; i++) {
        let idx = tid * ELEMS_PER_THREAD + i;
        if idx < MAX_SLOTS {
            let prefix_quads = shared_data[idx];
            let quad_count = mesh_counts[idx];
            let base = idx * 5u;
            mesh_offset_table[base]      = prefix_quads * 4u;  // vertex_offset
            mesh_offset_table[base + 1u] = quad_count * 4u;    // vertex_count
            mesh_offset_table[base + 2u] = prefix_quads * 6u;  // index_offset
            mesh_offset_table[base + 3u] = quad_count * 6u;    // index_count
            mesh_offset_table[base + 4u] = 0u;                  // write_counter (zeroed for Pass 3)
        }
    }
}
