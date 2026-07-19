// The hover-cursor ring (brush-editing Stage D, docs/design/brush-editing/04):
// a uniform at @9, zero by default (inactive — every render is then
// byte-identical to a cursor-free build, which the image tests pin). When
// active, hit voxels whose centre sits within ±0.75 voxels of the brush
// sphere's surface tint toward the highlight — a screen-stable ring wrapped
// onto the actual surface, honest about what the stamp will cover.
struct Cursor {
    pos: vec3<f32>,
    radius: f32,
    normal: vec3<f32>,
    enabled: f32,
}
@group(0) @binding(9) var<uniform> cursor: Cursor;

fn cursor_tint(color: vec3<f32>, world: vec3<u32>) -> vec3<f32> {
    if (cursor.enabled < 0.5) {
        return color;
    }
    let p = vec3<f32>(world) + vec3<f32>(0.5);
    let band = abs(distance(p, cursor.pos) - cursor.radius);
    if (band < 0.75) {
        return mix(color, vec3<f32>(1.0, 0.85, 0.25), 0.6);
    }
    return color;
}
