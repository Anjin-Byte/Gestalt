# WebGPU Profiling Playbook

## RenderDoc capture (Chrome)

1. Install RenderDoc (https://renderdoc.org) and Chrome Canary or a current Chrome with WebGPU enabled.
2. Launch Chrome with WebGPU enabled. Current guidance is to use:

   chrome --enable-unsafe-webgpu --enable-dawn-features=disallow_unsafe_apis

   Reference: Toji's WebGPU capture notes (https://toji.dev/webgpu/).
3. Load the local dev server (for example, http://localhost:5173).
4. In RenderDoc, capture a frame from the Chrome process.
5. Trigger a render frame (click Run Module in the UI) and capture.
6. Inspect the pass list and buffers in RenderDoc to trace vertex/instance data.

## WebGPU error scopes (module debugging)

- Wrap GPU calls with device.pushErrorScope("validation") and device.popErrorScope() to surface errors in module init/run.
- Use scopes sparingly around suspect sections to avoid masking errors.
