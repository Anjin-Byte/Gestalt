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
pub fn generate_spiral_points(count: u32, turns: f32, radius: f32) -> Object {
    let mut positions: Vec<f32> = Vec::with_capacity((count * 3) as usize);
    let mut indices: Vec<u32> = Vec::with_capacity(count as usize);

    for i in 0..count {
        let t = i as f32 / count.max(1) as f32;
        let angle = turns * std::f32::consts::TAU * t;
        let r = radius * t;
        let x = r * angle.cos();
        let y = (t - 0.5) * radius;
        let z = r * angle.sin();
        positions.push(x);
        positions.push(y);
        positions.push(z);
        indices.push(i);
    }

    let positions = Float32Array::from(positions.as_slice());
    let indices = Uint32Array::from(indices.as_slice());

    let object = Object::new();
    Reflect::set(&object, &JsValue::from_str("positions"), &positions).ok();
    Reflect::set(&object, &JsValue::from_str("indices"), &indices).ok();
    object
}
