// Palette/materials per-voxel colour lookup: declares the palette bindings (@8/@9)
// and the `fetch_albedo` the shared `gbuffer.wgsl` calls. No entry point. The consts
// MAT_STRIDE_W / MAT_PAL_OFF / MAT_IDX_OFF are injected ahead of this module (pinned
// to `voxel_core::palette` STRIDE_W / PAL_OFF / IDX_OFF). Concatenated after
// `traversal.wgsl` and before `gbuffer.wgsl`. `read_material` is kept body-identical
// to main's forward `render.wgsl` (the parity reference).

@group(0) @binding(8) var<storage, read> leaf_mat: array<u32>;
@group(0) @binding(9) var<storage, read> material_table: array<u32>;

// Decode the hit voxel's 4-bit palette index from its leaf's packed slot, then the
// inline `u16` palette → global material id. `bits == 0` (single-material leaf) skips
// the index array — a `(1u << 0u) - 1u` mask is degenerate, so the branch is
// mandatory. Straddle handling reads the next 32-bit word (vs the CPU reference's 64).
fn read_material(s: u32, m: u32) -> u32 {
    let base = s * MAT_STRIDE_W;
    let bits = leaf_mat[base] & 0xFu; // bits_per_voxel
    var pi: u32 = 0u;
    if (bits != 0u) {
        let off = m * bits;
        let iw = base + MAT_IDX_OFF + (off >> 5u);
        let pos = off & 31u;
        pi = leaf_mat[iw] >> pos;
        if (pos + bits > 32u) {
            pi = pi | (leaf_mat[iw + 1u] << (32u - pos));
        }
        pi = pi & ((1u << bits) - 1u);
    }
    let pal_word = leaf_mat[base + MAT_PAL_OFF + (pi >> 1u)];
    return (pal_word >> (16u * (pi & 1u))) & 0xFFFFu; // u16 low/high half-select
}

// The albedo `gbuffer.wgsl` writes for a hit. Global-0 (the reserved sentinel slot /
// unmaterialed voxel) renders as main's flat `world/n` tint — computed here in the
// g-buffer (so composite needs no palette-specific branch); a real id reads its sRGB
// RGBA8 colour from the table. `camera` is declared in the concatenated gbuffer.wgsl.
fn fetch_albedo(leaf: u32, vox: u32, world: vec3<u32>) -> vec4<f32> {
    let mat_id = read_material(leaf, vox);
    if (mat_id == 0u) {
        return vec4<f32>(
            f32(world.x) / camera.n,
            f32(world.y) / camera.n,
            f32(world.z) / camera.n,
            1.0,
        );
    }
    return unpack4x8unorm(material_table[mat_id]);
}
