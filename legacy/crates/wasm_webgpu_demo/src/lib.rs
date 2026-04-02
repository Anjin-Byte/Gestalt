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
pub fn wgsl_source() -> String {
    let source = r#"
struct Params {
  count: u32,
  radius: f32,
  _pad0: f32,
  _pad1: f32,
};

@group(0) @binding(0) var<storage, read_write> positions: array<vec4<f32>>;
@group(0) @binding(1) var<uniform> params: Params;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
  let idx = id.x;
  if (idx >= params.count) {
    return;
  }
  let t = f32(idx) / max(1.0, f32(params.count - 1u));
  let angle = t * 6.28318530718;
  let x = cos(angle) * params.radius;
  let y = sin(angle) * params.radius;
  positions[idx] = vec4<f32>(x, y, 0.0, 1.0);
}
"#;
    source.to_string()
}
