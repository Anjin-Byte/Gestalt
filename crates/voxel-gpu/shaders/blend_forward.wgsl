// Forward transparency pass (Stage C). Concatenated *after* `traversal.wgsl`, so the
// shared bindings (`nodes`@0, `leaf_words`@1, `leaf_bounds`@2) and the DDA helpers
// (`make_frame`/`walker_step`/`leaf_bit`/`leaf_reaches`/`morton8`/`child_*`) are in
// scope. A fork of main's `render_truecolor_blend.wgsl`: same front-to-back
// premultiplied-OVER march, but it composites the transparent voxels **over GTAO's
// already-lit opaque result** (`lit_color`) instead of a flat sky, clips them at the
// stored opaque depth, and writes the HDR `color_blended` TAA then reads. Dispatched
// only for scenes with `has_transparency`. `PER_CHUNK`/`MAX_BLEND` are injected ahead
// by `buffers::blend_forward_shader_source`. Transparents are UNLIT (parity w/ main).

struct Camera {
    eye: vec3<f32>,
    tan: f32,
    forward: vec3<f32>,
    aspect: f32,
    right: vec3<f32>,
    n: f32,
    up: vec3<f32>,
    _pad: f32,
    dims: vec4<u32>, // width, height, k, flags
}

@group(0) @binding(3) var<uniform> camera: Camera;
// Per-leaf colour base + N_MAX=3 colour chunks — the same truecolor buffers the
// g-buffer binds (this pass runs only for has_transparency ⟹ Truecolor scenes).
@group(0) @binding(5) var<storage, read> leaf_color_base: array<u32>;
@group(0) @binding(6) var<storage, read> leaf_color_0: array<u32>;
@group(0) @binding(7) var<storage, read> leaf_color_1: array<u32>;
@group(0) @binding(8) var<storage, read> leaf_color_2: array<u32>;
// The lit opaque result (composite output, incl. sky) we composite over, the opaque
// depth we clip against, and the HDR target TAA reads in place of `color`.
@group(0) @binding(9) var lit_color: texture_2d<f32>;
@group(0) @binding(10) var gbuf_depth: texture_2d<f32>;
@group(0) @binding(11) var color_blended: texture_storage_2d<rgba16float, write>;

// === colour read (copied verbatim from render_truecolor_blend.wgsl) ===

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

// The unpacked colour (sRGB RGBA8 → [0,1], A in .w) of the occupied voxel.
fn voxel_color(slot: u32, vox: u32) -> vec4<f32> {
    let g = leaf_color_base[slot] + leaf_color_rank(slot, vox);
    return unpack4x8unorm(read_leaf_color(g));
}

// Eye-space ray-box t_enter against the unit voxel cube at min corner `vmin` — copied
// from gbuffer.wgsl so the depth gate is ULP-identical to the g-buffer's stored depth.
fn voxel_t_enter(o: vec3<f32>, d: vec3<f32>, vmin: vec3<f32>) -> f32 {
    let inv = vec3<f32>(1.0) / d;
    let t0 = (vmin - o) * inv;
    let t1 = (vmin + vec3<f32>(1.0) - o) * inv;
    let tmin = min(t0, t1);
    return max(max(tmin.x, tmin.y), tmin.z);
}

// === front-to-back compositing traversal (fork of traverse_and_composite) ===
//
// Marches the DDA front-to-back; at each transparent voxel (TRANSPARENCY_BIT) it
// decodes the colour to linear and alpha-composites premultiplied OVER. Returns the
// PREMULTIPLIED accumulated linear colour in `.rgb` + coverage in `.a`; the caller
// composites the lit backdrop under `(1 - .a)`. Stops at the first OPAQUE voxel (it
// is already lit in the backdrop — do NOT re-composite it) and at any voxel at/behind
// the stored `opaque_depth`. Bounded by `acc_a >= 0.99` and `MAX_BLEND`.
fn traverse_and_composite(o: vec3<f32>, d: vec3<f32>, n: f32, k: u32, opaque_depth: f32) -> vec4<f32> {
    var acc = vec3<f32>(0.0, 0.0, 0.0); // premultiplied, LINEAR
    var acc_a = 0.0;
    var depth = 0u;

    // Grid-clip (f32 slab) against [0, n]³ — identical to traverse_ray.
    var t_near = -BIG;
    var t_far = BIG;
    var missed = false;
    if (d.x == 0.0) { if (o.x < 0.0 || o.x > n) { missed = true; } }
    else { let inv = 1.0 / d.x; var a = (0.0 - o.x) * inv; var b = (n - o.x) * inv; if (a > b) { let t = a; a = b; b = t; } t_near = max(t_near, a); t_far = min(t_far, b); }
    if (d.y == 0.0) { if (o.y < 0.0 || o.y > n) { missed = true; } }
    else { let inv = 1.0 / d.y; var a = (0.0 - o.y) * inv; var b = (n - o.y) * inv; if (a > b) { let t = a; a = b; b = t; } t_near = max(t_near, a); t_far = min(t_far, b); }
    if (d.z == 0.0) { if (o.z < 0.0 || o.z > n) { missed = true; } }
    else { let inv = 1.0 / d.z; var a = (0.0 - o.z) * inv; var b = (n - o.z) * inv; if (a > b) { let t = a; a = b; b = t; } t_near = max(t_near, a); t_far = min(t_far, b); }

    if (missed || t_near > t_far || t_far < 0.0) {
        return vec4<f32>(acc, acc_a);
    }
    let t_entry = max(t_near, 0.0);

    var root_level = 1u;
    if (k > 0u) {
        root_level = k + 1u;
    }
    var cur = make_frame(o, d, 0u, root_level, vec3<u32>(0u, 0u, 0u), t_entry);
    var stack: array<Frame, 6>;
    var sp = 0u;

    for (var iter = 0u; iter < 200000u; iter = iter + 1u) {
        if (cur.level == 1u) {
            let v = cur.cell;
            if (leaf_bit(cur.node, v)) {
                // Read the voxel's RGBA once (.a = per-voxel alpha). The STOP is
                // PER-VOXEL (alpha==255 ⇒ opaque), matching the skip-transparent
                // g-buffer's predicate so both passes stop at the SAME voxel — correct
                // even inside a leaf that mixes opaque + transparent voxels (the
                // per-leaf TRANSPARENCY_BIT would misclassify those).
                let c = voxel_color(cur.node, morton8(v & vec3<u32>(7u)));
                if (c.a >= 1.0) {
                    // First OPAQUE voxel — already lit in lit_color (the g-buffer
                    // captured this same voxel). STOP, do not composite it.
                    return vec4<f32>(acc, acc_a);
                }
                // Depth gate — now a redundant ULP-coincidence guard: the g-buffer
                // writes the DEEP opaque depth, so it bounds correctly BEHIND the
                // transparents, not in front of them.
                let vmin = vec3<f32>(cur.origin + cur.cell);
                let vz = voxel_t_enter(o, d, vmin) * dot(d, camera.forward);
                if (opaque_depth > 0.0 && vz >= opaque_depth) {
                    return vec4<f32>(acc, acc_a);
                }
                // Semi-transparent voxel: decode to linear, premultiplied OVER.
                let lin = pow(c.rgb, vec3<f32>(2.2));
                let wgt = (1.0 - acc_a) * c.a;
                acc = acc + wgt * lin;
                acc_a = acc_a + wgt;
                depth = depth + 1u;
                if (acc_a >= 0.99 || depth >= MAX_BLEND) {
                    return vec4<f32>(acc, acc_a);
                }
                // fall through to step past this voxel
            }
            if (walker_step(&cur)) { continue; }
            loop {
                if (sp == 0u) { return vec4<f32>(acc, acc_a); }
                sp = sp - 1u;
                cur = stack[sp];
                if (walker_step(&cur)) { break; }
            }
        } else {
            let c = cur.cell;
            let bit = child_bit(c);
            let node = nodes[cur.node];
            let child_level = cur.level - 1u;
            let size = cell_size_of(cur.level);
            let child_origin = cur.origin + c * size;
            var descend = has_child(node, bit);
            var slot = 0u;
            if (descend) {
                slot = child_slot(node, bit);
                if (child_level == 1u) {
                    descend = leaf_reaches(slot, o, d, child_origin, cur.t_entry);
                }
            }
            if (descend) {
                stack[sp] = cur;
                sp = sp + 1u;
                cur = make_frame(o, d, slot, child_level, child_origin, cur.t_entry);
            } else if (!walker_step(&cur)) {
                loop {
                    if (sp == 0u) { return vec4<f32>(acc, acc_a); }
                    sp = sp - 1u;
                    cur = stack[sp];
                    if (walker_step(&cur)) { break; }
                }
            }
        }
    }
    return vec4<f32>(acc, acc_a);
}

@compute @workgroup_size(8, 8)
fn blend_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let width = camera.dims.x;
    let height = camera.dims.y;
    if (gid.x >= width || gid.y >= height) {
        return;
    }
    let coord = vec2<i32>(i32(gid.x), i32(gid.y));
    // The lit opaque result (or sky) we composite the transparents over, and the
    // opaque depth the march clips against.
    let backdrop = textureLoad(lit_color, coord, 0).rgb;
    let opaque_depth = textureLoad(gbuf_depth, coord, 0).x;

    let w = f32(width);
    let h = f32(height);
    let ndc_x = ((f32(gid.x) + 0.5) / w * 2.0 - 1.0) * camera.tan * camera.aspect;
    let ndc_y = (1.0 - (f32(gid.y) + 0.5) / h * 2.0) * camera.tan;
    let dir = normalize(camera.forward + camera.right * ndc_x + camera.up * ndc_y);

    let acc = traverse_and_composite(camera.eye, dir, camera.n, camera.dims.z, opaque_depth);
    let rgb = acc.rgb + (1.0 - acc.a) * backdrop;
    textureStore(color_blended, vec2<u32>(gid.x, gid.y), vec4<f32>(rgb, 1.0));
}
