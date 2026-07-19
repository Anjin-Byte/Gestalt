// Skip-transparent g-buffer (Stage C). Concatenated ONLY into the truecolor g-buffer
// assembly, AFTER traversal.wgsl + color_lookup.wgsl + gbuffer.wgsl — so HitResult /
// Frame / make_frame / walker_step / leaf_bit / leaf_reaches / morton8 (traversal),
// fetch_albedo (color_lookup, bindings @8..11), and voxel_face / SHADOW_* /
// gbuffer_ray / shade_gbuffer_hit (gbuffer) are all in scope.
//
// `traverse_ray_opaque` is a fork of `traverse_ray` whose SOLE divergence is the
// leaf-hit branch: it reads PER-VOXEL alpha and SKIPS transparent voxels (baked
// alpha<255 ⇒ a BLEND voxel), returning the first OPAQUE voxel (alpha==255) as the
// surface. For has_transparency scenes this makes the g-buffer capture the OPAQUE
// backdrop (so GTAO/shadows light it) and the blend pass composites the transparents
// in front of it. The shared `traverse_ray` is left byte-for-byte untouched.

fn traverse_ray_opaque(o: vec3<f32>, d: vec3<f32>, n: f32, k: u32) -> HitResult {
    // Grid-clip (f32 slab) against [0, n]³ — verbatim from traverse_ray.
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
        return HitResult(vec3<u32>(0u, 0u, 0u), 0u, 0u, 0u);
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
                let org = cur.origin;
                let vox = morton8(v & vec3<u32>(7u)); // SAME intra-leaf order leaf_bit uses
                let world = vec3<u32>(org.x + v.x, org.y + v.y, org.z + v.z);
                // Per-VOXEL alpha — NOT the per-leaf TRANSPARENCY_BIT (a leaf can mix
                // opaque + transparent voxels). Opaque voxels are baked a==255, and
                // unpack4x8unorm(0xFF) is exactly 1.0, so `>= 1.0` ⟺ opaque; alpha<1 is
                // a BLEND voxel ⇒ skip it and keep marching for the surface behind.
                if (fetch_albedo(cur.node, vox, world).w >= 1.0) {
                    return HitResult(world, 1u, cur.node, vox); // first opaque ⇒ backdrop
                }
                // transparent: fall through to step past this voxel and keep marching
            }
            if (walker_step(&cur)) { continue; }
            // Ascend: pop a parent into `cur` and step it.
            loop {
                if (sp == 0u) { return HitResult(vec3<u32>(0u, 0u, 0u), 0u, 0u, 0u); }
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
                    if (sp == 0u) { return HitResult(vec3<u32>(0u, 0u, 0u), 0u, 0u, 0u); }
                    sp = sp - 1u;
                    cur = stack[sp];
                    if (walker_step(&cur)) { break; }
                }
            }
        }
    }
    return HitResult(vec3<u32>(0u, 0u, 0u), 0u, 0u, 0u);
}

@compute @workgroup_size(8, 8)
fn gbuffer_opaque_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= camera.dims.x || gid.y >= camera.dims.y) {
        return;
    }
    let dir = gbuffer_ray(gid);
    let hit = traverse_ray_opaque(camera.eye, dir, camera.n, camera.dims.z);
    shade_gbuffer_hit(hit, dir, gid);
}
