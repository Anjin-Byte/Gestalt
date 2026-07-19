// GPU colour bake — a transcription of `gpu/colorbake.rs::reference_bake` and
// the pinned sampling chain in `bake.rs` (closest point → barycentric → UV →
// manual bilinear in linear space via the uploaded sRGB tables → factor tint →
// table-search sRGB encode). One workgroup per leaf (256 threads × 2 morton
// slots); the same-material owner constraint, the fall-back-to-all rule, the
// min-`tri_index` tie break, and the `color_base + rank` output addressing all
// mirror the CPU reference exactly — the differential tests compare
// word-for-word. Do not edit without the CPU sources; this file has no
// independent semantics.

struct Uniforms {
    // First leaf of this dispatch chunk and the number of leaves in it.
    leaf_offset: u32,
    leaf_count: u32,
    // 1 = the same-material owner constraint applies.
    material_filter: u32,
    // aux-word offsets of the sRGB tables (texture meta sits at 0).
    decode_off: u32,
    bounds_off: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

// Mirrors `PackedTri` (96 bytes).
struct PackedTri {
    v0: vec3<f32>,
    tex: u32,
    v1: vec3<f32>,
    flags: u32,
    v2: vec3<f32>,
    global_mat: u32,
    uv: array<vec2<f32>, 3>,
    tri_index: u32,
    pad: u32,
    factor: vec4<f32>,
}

// Mirrors `PackedLeaf` (32 bytes).
struct PackedLeaf {
    origin: vec3<u32>,
    cand_lo: u32,
    cand_hi: u32,
    color_base: u32,
    pad0: u32,
    pad1: u32,
}

@group(0) @binding(0) var<storage, read> texels: array<u32>;
// aux = [texture meta (4 words each: offset,w,h,pad) | sRGB decode f32 bits | encode-bound f32 bits]
@group(0) @binding(1) var<storage, read> aux: array<u32>;
@group(0) @binding(2) var<storage, read> tris: array<PackedTri>;
@group(0) @binding(3) var<storage, read> cand_indices: array<u32>;
@group(0) @binding(4) var<storage, read> leaves: array<PackedLeaf>;
@group(0) @binding(5) var<storage, read> leaf_words: array<u32>;
@group(0) @binding(6) var<storage, read> leaf_mats: array<u32>;
@group(0) @binding(7) var<storage, read_write> out_colors: array<u32>;
@group(0) @binding(8) var<uniform> U: Uniforms;

const FLAG_WRAP_S_REPEAT: u32 = 1u;
const FLAG_WRAP_T_REPEAT: u32 = 2u;
const FLAG_BLEND: u32 = 4u;
const NO_TEXTURE: u32 = 0xffffffffu;
const MAX_SUPERSAMPLE: u32 = 8u;
const MISSING_MAGENTA: u32 = 0xffff00ffu;

// ---- sRGB tables (bit-identical to the CPU LazyLock tables) ----

fn srgb_to_linear(b: u32) -> f32 {
    return bitcast<f32>(aux[U.decode_off + b]);
}

// `SRGB_ENCODE_BOUNDS.partition_point(|&bound| bound <= c)` — 255 entries.
fn linear_to_srgb_u8(c0: f32) -> u32 {
    let c = clamp(c0, 0.0, 1.0);
    var lo = 0u;
    var hi = 255u;
    while (lo < hi) {
        let mid = (lo + hi) / 2u;
        if (bitcast<f32>(aux[U.bounds_off + mid]) <= c) {
            lo = mid + 1u;
        } else {
            hi = mid;
        }
    }
    return lo;
}

// ---- geometry (Ericson closest point; barycentric re-solve) ----

fn closest_point_on_triangle(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>, c: vec3<f32>) -> vec3<f32> {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if (d1 <= 0.0 && d2 <= 0.0) {
        return a;
    }
    let bp = p - b;
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if (d3 >= 0.0 && d4 <= d3) {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if (vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0) {
        let v = d1 / (d1 - d3);
        return a + ab * v;
    }
    let cp = p - c;
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if (d6 >= 0.0 && d5 <= d6) {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if (vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0) {
        let w = d2 / (d2 - d6);
        return a + ac * w;
    }
    let va = d3 * d6 - d5 * d4;
    if (va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0) {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return b + (c - b) * w;
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    return a + ab * v + ac * w;
}

fn barycentric(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>, c: vec3<f32>) -> vec3<f32> {
    let v0 = b - a;
    let v1 = c - a;
    let v2 = p - a;
    let d00 = dot(v0, v0);
    let d01 = dot(v0, v1);
    let d11 = dot(v1, v1);
    let d20 = dot(v2, v0);
    let d21 = dot(v2, v1);
    let denom = d00 * d11 - d01 * d01;
    if (abs(denom) < 1e-20) {
        return vec3<f32>(1.0, 0.0, 0.0);
    }
    let v = (d11 * d20 - d01 * d21) / denom;
    let w = (d00 * d21 - d01 * d20) / denom;
    return vec3<f32>(1.0 - v - w, v, w);
}

fn clamp_bary(b: vec3<f32>) -> vec3<f32> {
    let m = max(b, vec3<f32>(0.0));
    let sum = m.x + m.y + m.z;
    if (sum > 1e-9) {
        return m / sum;
    }
    return vec3<f32>(1.0, 0.0, 0.0);
}

// ---- manual bilinear over the packed texel buffer ----

fn wrap_uv(u: f32, repeat: bool) -> f32 {
    if (repeat) {
        return u - floor(u); // fract, always in [0,1)
    }
    return clamp(u, 0.0, 1.0);
}

fn wrap_texel(i: i32, dim: u32, repeat: bool) -> u32 {
    let d = i32(dim);
    if (repeat) {
        return u32(((i % d) + d) % d); // rem_euclid
    }
    return u32(clamp(i, 0, d - 1));
}

fn tap(tex_off: u32, tex_w: u32, x: u32, y: u32) -> vec4<f32> {
    let t = texels[tex_off + y * tex_w + x];
    return vec4<f32>(
        srgb_to_linear(t & 0xffu),
        srgb_to_linear((t >> 8u) & 0xffu),
        srgb_to_linear((t >> 16u) & 0xffu),
        f32((t >> 24u) & 0xffu) / 255.0,
    );
}

fn sample_bilinear_linear(tex_index: u32, uv: vec2<f32>, flags: u32) -> vec4<f32> {
    let meta_base = tex_index * 4u;
    let tex_off = aux[meta_base];
    let tex_w = aux[meta_base + 1u];
    let tex_h = aux[meta_base + 2u];
    let rep_s = (flags & FLAG_WRAP_S_REPEAT) != 0u;
    let rep_t = (flags & FLAG_WRAP_T_REPEAT) != 0u;

    let u = wrap_uv(uv.x, rep_s);
    let v = wrap_uv(uv.y, rep_t);
    // Texel-centre convention: texel centre i maps to (i+0.5)/dim.
    let fx = u * f32(tex_w) - 0.5;
    let fy = v * f32(tex_h) - 0.5;
    let x0f = floor(fx);
    let y0f = floor(fy);
    let tx = fx - x0f;
    let ty = fy - y0f;
    let x0 = i32(x0f);
    let y0 = i32(y0f);

    let c00 = tap(tex_off, tex_w, wrap_texel(x0, tex_w, rep_s), wrap_texel(y0, tex_h, rep_t));
    let c10 = tap(tex_off, tex_w, wrap_texel(x0 + 1, tex_w, rep_s), wrap_texel(y0, tex_h, rep_t));
    let c01 = tap(tex_off, tex_w, wrap_texel(x0, tex_w, rep_s), wrap_texel(y0 + 1, tex_h, rep_t));
    let c11 = tap(tex_off, tex_w, wrap_texel(x0 + 1, tex_w, rep_s), wrap_texel(y0 + 1, tex_h, rep_t));

    let a = c00 * (1.0 - tx) + c10 * tx;
    let b = c01 * (1.0 - tx) + c11 * tx;
    return a * (1.0 - ty) + b * ty;
}

// ---- encode (linear → sRGB RGBA8 word, R low) ----

fn encode_color(linear: vec4<f32>, factor: vec4<f32>) -> u32 {
    let r = linear_to_srgb_u8(linear.x * factor.x);
    let g = linear_to_srgb_u8(linear.y * factor.y);
    let b = linear_to_srgb_u8(linear.z * factor.z);
    let a = u32(clamp(linear.w * factor.w, 0.0, 1.0) * 255.0 + 0.5);
    return r | (g << 8u) | (b << 16u) | (a << 24u);
}

fn normalize_or_zero(v: vec3<f32>) -> vec3<f32> {
    let l = length(v);
    if (l > 0.0) {
        return v / l;
    }
    return vec3<f32>(0.0);
}

// `expected_color_filtered`: single tap when magnified, stratified box
// supersample over the voxel cell when minified.
fn expected_color_filtered(centre: vec3<f32>, ti: u32) -> u32 {
    let t = tris[ti];
    if (t.tex == NO_TEXTURE) {
        return encode_color(vec4<f32>(1.0), t.factor);
    }
    let p = closest_point_on_triangle(centre, t.v0, t.v1, t.v2);

    let meta_base = t.tex * 4u;
    let tex_w = aux[meta_base + 1u];
    let tex_h = aux[meta_base + 2u];
    let e1 = t.v1 - t.v0;
    let e2 = t.v2 - t.v0;
    let l1 = length(e1);
    let l2 = length(e2);
    var d1 = 0.0;
    if (l1 > 1e-9) {
        d1 = length(t.uv[1] - t.uv[0]) / l1;
    }
    var d2 = 0.0;
    if (l2 > 1e-9) {
        d2 = length(t.uv[2] - t.uv[0]) / l2;
    }
    let footprint = max(d1, d2) * f32(max(tex_w, tex_h));
    let s = clamp(u32(ceil(footprint + 0.5)), 1u, MAX_SUPERSAMPLE);

    if (s <= 1u) {
        let b = barycentric(p, t.v0, t.v1, t.v2);
        let uv = t.uv[0] * b.x + t.uv[1] * b.y + t.uv[2] * b.z;
        return encode_color(sample_bilinear_linear(t.tex, uv, t.flags), t.factor);
    }

    let n_axis = normalize_or_zero(cross(e1, e2));
    let t1 = normalize_or_zero(e1);
    let t2 = cross(n_axis, t1);
    let inv = 1.0 / f32(s);
    var acc = vec4<f32>(0.0);
    for (var i = 0u; i < s; i++) {
        for (var j = 0u; j < s; j++) {
            let oi = (f32(i) + 0.5) * inv - 0.5;
            let oj = (f32(j) + 0.5) * inv - 0.5;
            let sp = p + t1 * oi + t2 * oj;
            let b = clamp_bary(barycentric(sp, t.v0, t.v1, t.v2));
            let uv = t.uv[0] * b.x + t.uv[1] * b.y + t.uv[2] * b.z;
            acc += sample_bilinear_linear(t.tex, uv, t.flags);
        }
    }
    let n = f32(s * s);
    return encode_color(acc / n, tris[ti].factor);
}

// ---- the per-voxel owner pick (argmin, min-tri_index tie, material filter) ----

struct Owner {
    found: bool,
    tri: u32, // index into `tris`
}

fn pick_owner_pass(centre: vec3<f32>, lo: u32, hi: u32, want_mat: u32, filtered: bool) -> Owner {
    var best_d = 0.0;
    var best_tri = 0u;
    var found = false;
    for (var ci = lo; ci < hi; ci++) {
        let ti = cand_indices[ci];
        let t = tris[ti];
        if (filtered && t.global_mat != want_mat) {
            continue;
        }
        let p = closest_point_on_triangle(centre, t.v0, t.v1, t.v2);
        let d = dot(p - centre, p - centre);
        var better = !found;
        if (found) {
            better = d < best_d || (d == best_d && t.tri_index < best_tri);
        }
        if (better) {
            best_d = d;
            best_tri = t.tri_index;
            found = true;
        }
    }
    return Owner(found, best_tri);
}

// Intra-brick Morton decode (3 bits per axis: x bits 0,3,6; y 1,4,7; z 2,5,8).
fn brick_local(m: u32) -> vec3<u32> {
    let x = (m & 1u) | ((m >> 2u) & 2u) | ((m >> 4u) & 4u);
    let y = ((m >> 1u) & 1u) | ((m >> 3u) & 2u) | ((m >> 5u) & 4u);
    let z = ((m >> 2u) & 1u) | ((m >> 4u) & 2u) | ((m >> 6u) & 4u);
    return vec3<u32>(x, y, z);
}

// Masked popcount rank — the frozen `leaf_color_rank` addressing.
fn occupied_rank(word_base: u32, m: u32) -> u32 {
    var rank = 0u;
    let full = m >> 5u;
    for (var w = 0u; w < full; w++) {
        rank += countOneBits(leaf_words[word_base + w]);
    }
    let rem = m & 31u;
    if (rem > 0u) {
        rank += countOneBits(leaf_words[word_base + full] & ((1u << rem) - 1u));
    }
    return rank;
}

fn bake_voxel(leaf_index: u32, m: u32) {
    let word_base = leaf_index * 16u;
    if ((leaf_words[word_base + (m >> 5u)] & (1u << (m & 31u))) == 0u) {
        return;
    }
    let leaf = leaves[leaf_index];
    let local = brick_local(m);
    let centre = vec3<f32>(
        f32(leaf.origin.x + local.x) + 0.5,
        f32(leaf.origin.y + local.y) + 0.5,
        f32(leaf.origin.z + local.z) + 0.5,
    );

    var owner = Owner(false, 0u);
    if (U.material_filter != 0u) {
        let mat_word = leaf_mats[leaf_index * 256u + (m >> 1u)];
        let vox_mat = (mat_word >> ((m & 1u) * 16u)) & 0xffffu;
        owner = pick_owner_pass(centre, leaf.cand_lo, leaf.cand_hi, vox_mat, true);
    }
    if (!owner.found) {
        owner = pick_owner_pass(centre, leaf.cand_lo, leaf.cand_hi, 0u, false);
    }

    var color = MISSING_MAGENTA;
    if (owner.found) {
        color = expected_color_filtered(centre, owner.tri);
        // Force opaque alpha unless the owner material is BLEND.
        if ((tris[owner.tri].flags & FLAG_BLEND) == 0u) {
            color = (color & 0x00ffffffu) | 0xff000000u;
        }
    }
    out_colors[leaf.color_base + occupied_rank(word_base, m)] = color;
}

@compute @workgroup_size(256)
fn bake(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) lid: u32) {
    let leaf_index = U.leaf_offset + wid.x;
    if (wid.x >= U.leaf_count) {
        return;
    }
    // 512 morton slots on 256 threads: each thread takes m and m + 256.
    bake_voxel(leaf_index, lid);
    bake_voxel(leaf_index, lid + 256u);
}
