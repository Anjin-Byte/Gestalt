// Build Indirect Draw Args
//
// Reads mesh_offset_table + visibility. Writes DrawIndexedIndirect structs.
// Called twice per frame in two-pass occlusion:
//   Interim: reads pass1_visibility → indirect for depth prepass 2
//   Final:   reads merged visibility → indirect for color pass + next frame

const MAX_SLOTS: u32 = 4096u;
const INDIRECT_STRIDE: u32 = 5u;

@group(0) @binding(0) var<storage, read>       mesh_offset_table: array<u32>;
@group(0) @binding(1) var<storage, read_write> indirect_buf:      array<u32>;
@group(0) @binding(2) var<storage, read>       visibility:        array<u32>;

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) gid: vec3u) {
    let slot = gid.x;
    if slot >= MAX_SLOTS { return; }

    let ot_base = slot * 5u;
    let vert_offset = mesh_offset_table[ot_base];
    let idx_offset  = mesh_offset_table[ot_base + 2u];
    let idx_count   = mesh_offset_table[ot_base + 3u];

    let vis = visibility[slot];

    let ind_base = slot * INDIRECT_STRIDE;
    indirect_buf[ind_base]      = idx_count;
    indirect_buf[ind_base + 1u] = select(0u, 1u, idx_count > 0u && vis != 0u);
    indirect_buf[ind_base + 2u] = idx_offset;
    indirect_buf[ind_base + 3u] = vert_offset;
    indirect_buf[ind_base + 4u] = 0u;
}
