// Truecolor per-voxel colour lookup: declares the truecolor colour bindings (@8..11)
// and the `fetch_albedo` the shared `gbuffer.wgsl` calls (plus its helper bodies). No
// entry point. `const PER_CHUNK` is injected ahead of this module; concatenated after
// `traversal.wgsl` (so `leaf_words` is in scope) and before `gbuffer.wgsl`. Bodies
// kept identical to the forward `render_truecolor.wgsl` (the parity reference) so the
// deferred g-buffer resolves a voxel's colour the same way.
//
// `leaf_color_page` serves BOTH color representations (the same duality as the
// forward shader): a static build binds the per-leaf prefix-sum base array (a
// degenerate one-page-per-leaf pool), an editable paged build binds the page
// table — the read `page[leaf] + rank` is byte-identical either way.

@group(0) @binding(8) var<storage, read> leaf_color_page: array<u32>;
@group(0) @binding(9) var<storage, read> leaf_color_0: array<u32>;
@group(0) @binding(10) var<storage, read> leaf_color_1: array<u32>;
@group(0) @binding(11) var<storage, read> leaf_color_2: array<u32>;

// Frozen transcription of `LeafBrick::occupied_rank` (== leaf.rs `wgsl_rank`,
// parity-pinned): a 16-word masked popcount over the SAME `leaf_words` view
// `leaf_bit` reads (stride 16 u32/leaf). Counts occupied voxels with intra-brick
// Morton STRICTLY < `m`. NOT one `countOneBits` — a single-word mask is UB for
// `m >= 32`. The `rem == 0` skip is load-bearing: for `m` a multiple of 32 the
// partial word index would be `full` (16 at m=512, OOB); `m < 512` keeps
// `full <= 15`. Do NOT optimise it away.
fn leaf_color_rank(slot: u32, m: u32) -> u32 {
    let wbase = slot * 16u;
    let full = m >> 5u;
    var rank: u32 = 0u;
    for (var w: u32 = 0u; w < full; w = w + 1u) {
        rank = rank + countOneBits(leaf_words[wbase + w]);
    }
    let rem = m & 31u;
    if (rem > 0u) {
        rank = rank + countOneBits(leaf_words[wbase + full] & ((1u << rem) - 1u));
    }
    return rank;
}

// Chunk-select over N_MAX=3. The capability probe guarantees a valid global index
// `g < N * PER_CHUNK <= N_MAX * PER_CHUNK`, so `g / PER_CHUNK < N <= N_MAX` — the
// final arm is reached only for a real chunk 2, never a dummy-bound unused slot.
fn read_leaf_color(g: u32) -> u32 {
    let chunk = g / PER_CHUNK;
    let local = g % PER_CHUNK;
    if (chunk == 0u) {
        return leaf_color_0[local];
    }
    if (chunk == 1u) {
        return leaf_color_1[local];
    }
    return leaf_color_2[local];
}

// The albedo `gbuffer.wgsl` writes for a hit: the voxel's baked sRGB RGBA8 (R low),
// unpacked verbatim. `world` is unused here (the palette variant uses it for its
// global-0 fallback); the shared signature keeps `gbuffer.wgsl` variant-agnostic.
fn fetch_albedo(leaf: u32, vox: u32, world: vec3<u32>) -> vec4<f32> {
    let g = leaf_color_page[leaf] + leaf_color_rank(leaf, vox);
    return unpack4x8unorm(read_leaf_color(g));
}
