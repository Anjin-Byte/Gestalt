use js_sys::{Float32Array, Object, Reflect, Uint32Array};
use wasm_bindgen::prelude::*;
use web_sys::console;

#[wasm_bindgen]
pub fn init_logging() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub fn log_info(message: &str) {
    console::log_1(&message.into());
}

#[wasm_bindgen]
pub fn parse_obj(input: String) -> Object {
    let mut positions: Vec<f32> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("v ") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 4 {
                if let (Ok(x), Ok(y), Ok(z)) = (
                    parts[1].parse::<f32>(),
                    parts[2].parse::<f32>(),
                    parts[3].parse::<f32>(),
                ) {
                    positions.push(x);
                    positions.push(y);
                    positions.push(z);
                }
            }
        } else if trimmed.starts_with("f ") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 4 {
                let mut face_indices: Vec<u32> = Vec::new();
                for part in parts.iter().skip(1) {
                    let raw = part.split('/').next().unwrap_or("");
                    if let Ok(idx) = raw.parse::<i32>() {
                        if idx > 0 {
                            face_indices.push((idx - 1) as u32);
                        }
                    }
                }

                if face_indices.len() >= 3 {
                    let base = face_indices[0];
                    for i in 1..face_indices.len() - 1 {
                        indices.push(base);
                        indices.push(face_indices[i]);
                        indices.push(face_indices[i + 1]);
                    }
                }
            }
        }
    }

    let positions = Float32Array::from(positions.as_slice());
    let indices = Uint32Array::from(indices.as_slice());

    let object = Object::new();
    Reflect::set(&object, &JsValue::from_str("positions"), &positions).ok();
    Reflect::set(&object, &JsValue::from_str("indices"), &indices).ok();
    object
}

#[wasm_bindgen]
pub fn wgsl_source() -> String {
    let source = r#"
struct Params {
  count: u32,
  _pad0: u32,
  _pad1: u32,
  _pad2: u32,
};

@group(0) @binding(0) var<storage, read> input_positions: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> output_positions: array<vec4<f32>>;
@group(0) @binding(2) var<uniform> transform: mat4x4<f32>;
@group(0) @binding(3) var<uniform> params: Params;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
  let idx = id.x;
  if (idx >= params.count) {
    return;
  }
  let p = input_positions[idx];
  output_positions[idx] = transform * p;
}
"#;
    source.to_string()
}

#[wasm_bindgen]
pub fn transform_matrix(
    scale: f32,
    rot_x_deg: f32,
    rot_y_deg: f32,
    rot_z_deg: f32,
    translate_x: f32,
    translate_y: f32,
    translate_z: f32,
) -> Float32Array {
    let to_rad = |deg: f32| deg.to_radians();
    let (sx, cx) = to_rad(rot_x_deg).sin_cos();
    let (sy, cy) = to_rad(rot_y_deg).sin_cos();
    let (sz, cz) = to_rad(rot_z_deg).sin_cos();

    let s = [
        scale, 0.0, 0.0, 0.0, //
        0.0, scale, 0.0, 0.0, //
        0.0, 0.0, scale, 0.0, //
        0.0, 0.0, 0.0, 1.0, //
    ];

    let rx = [
        1.0, 0.0, 0.0, 0.0, //
        0.0, cx, sx, 0.0, //
        0.0, -sx, cx, 0.0, //
        0.0, 0.0, 0.0, 1.0, //
    ];

    let ry = [
        cy, 0.0, -sy, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        sy, 0.0, cy, 0.0, //
        0.0, 0.0, 0.0, 1.0, //
    ];

    let rz = [
        cz, sz, 0.0, 0.0, //
        -sz, cz, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0, //
    ];

    let t = [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        translate_x, translate_y, translate_z, 1.0, //
    ];

    let rs = multiply_mat4(&rz, &multiply_mat4(&ry, &multiply_mat4(&rx, &s)));
    let m = multiply_mat4(&t, &rs);

    Float32Array::from(m.as_slice())
}

fn multiply_mat4(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0f32; 16];
    for c in 0..4 {
        for r in 0..4 {
            let mut sum = 0.0f32;
            for k in 0..4 {
                sum += a[k * 4 + r] * b[c * 4 + k];
            }
            out[c * 4 + r] = sum;
        }
    }
    out
}
