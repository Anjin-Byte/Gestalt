/**
 * RendererController
 *
 * Main-thread WASM renderer lifecycle + frame loop.
 * Replaces RendererBridge (which used a Web Worker + OffscreenCanvas).
 *
 * Per ADR-0014: all GPU work runs on the main thread. The canvas is a
 * regular HTMLCanvasElement — no OffscreenCanvas, no worker transfer.
 * requestAnimationFrame is synchronized with the compositor's vsync,
 * eliminating the frame tearing defect from the worker-based approach.
 */

import initWasm, { Renderer } from "../wasm/wasm_renderer/wasm_renderer.js";
import { frameTimeline, diagCounters } from "$lib/stores/timeline";
import { rendererStatsStore } from "$lib/stores/rendererStats";

let renderer: Renderer | null = null;
let rafId = 0;
let statsCounter = 0;

/**
 * Initialize the WASM renderer on the main thread.
 * Call once after the canvas element is in the DOM.
 */
export async function init(canvas: HTMLCanvasElement): Promise<void> {
  await initWasm();

  const rect = canvas.getBoundingClientRect();
  const w = Math.round(rect.width * devicePixelRatio);
  const h = Math.round(rect.height * devicePixelRatio);
  canvas.width = w;
  canvas.height = h;

  renderer = await new Renderer(canvas, w, h);
  renderer.load_test_scene();

  console.log("[RendererController] initialized, test scene loaded");
  requestFrame();
}

function requestFrame() {
  rafId = requestAnimationFrame(renderFrame);
}

function renderFrame() {
  if (!renderer) return;

  try {
    renderer.render_frame();
  } catch (err) {
    console.error("[RendererController] render error:", err);
  }

  // ── Real per-pass CPU timing from WASM (not fabricated) ──
  const timings = renderer.get_pass_timings(); // [depth_ms, color_ms, total_ms]
  frameTimeline.push({
    totalMs: timings[2],
    passes: {
      "R-2 Depth Prepass": timings[0],
      "R-5 Color Pass": timings[1],
    },
  });

  // ── Stats + diagnostic counters (every 10 frames) ──
  statsCounter++;
  if (statsCounter >= 10) {
    statsCounter = 0;

    const camPos = renderer.get_camera_pos();
    const camDir = renderer.get_camera_dir();

    rendererStatsStore.set({
      frame: renderer.get_frame_index(),
      residentCount: renderer.get_resident_count(),
      renderMode: renderer.get_render_mode(),
      totalVoxels: renderer.get_total_voxels(),
      meshVerts: renderer.get_mesh_verts(),
      meshIndices: renderer.get_mesh_indices(),
      meshQuads: renderer.get_mesh_quads(),
      cameraPos: [camPos[0], camPos[1], camPos[2]],
      cameraDir: [camDir[0], camDir[1], camDir[2]],
      viewportWidth: renderer.get_viewport_width(),
      viewportHeight: renderer.get_viewport_height(),
      cameraFov: renderer.get_camera_fov(),
      cameraNear: renderer.get_camera_near(),
      cameraFar: renderer.get_camera_far(),
      cameraAspect: renderer.get_camera_aspect(),
      freeSlots: renderer.get_free_slot_count(),
      hasWireframe: renderer.get_has_wireframe(),
      backfaceCulling: renderer.get_backface_culling(),
      depthPrepassEnabled: renderer.get_depth_prepass_enabled(),
    });

    // Diagnostic counters — real values from WASM
    const diag = renderer.get_diag_counters(); // [summary_rebuilds, mesh_rebuilds]
    diagCounters.update({
      meshlets_culled: 0,           // Phase 5 — meshlets not implemented
      chunks_empty_skipped: 0,      // needs GPU readback
      version_mismatches: 0,        // Phase 4 — edit protocol
      summary_rebuilds: diag[0],    // I-3 dispatches this frame
      mesh_rebuilds: diag[1],       // R-1 dispatches this frame
      cascade_ray_hits: 0,          // Phase 3 — GI not implemented
    });
  }

  requestFrame();
}

/** Update camera position + direction. Call from orbit controls. */
export function setCamera(
  px: number, py: number, pz: number,
  dx: number, dy: number, dz: number,
) {
  renderer?.set_camera(px, py, pz, dx, dy, dz);
}

/** Resize the rendering surface. Call from ResizeObserver. */
export function resize(width: number, height: number) {
  renderer?.resize(width, height);
}

/** Switch render mode (Solid=0x00, Wireframe=0x02, Normals=0x04, Depth=0x10). */
export function setRenderMode(mode: number) {
  renderer?.set_render_mode(mode);
}

/** Set camera field of view in degrees. Clamped to [10, 120]. */
export function setFov(degrees: number) {
  renderer?.set_fov(degrees);
}

/** Toggle backface culling. */
export function setBackfaceCulling(enabled: boolean) {
  renderer?.set_backface_culling(enabled);
}

/** Toggle depth prepass (R-2). Disabling skips depth-only pass for debugging. */
export function setDepthPrepass(enabled: boolean) {
  renderer?.set_depth_prepass(enabled);
}

/** Load an OBJ model: parse → voxelize → upload → render. */
export function loadModel(objText: string, resolution: number) {
  if (!renderer) throw new Error("Renderer not initialized");
  renderer.load_obj_model(objText, resolution);
}

/** Toggle CPU mesh path (bypasses GPU mesh_rebuild compute shader). */
export function setUseCpuMesh(enabled: boolean) {
  renderer?.set_use_cpu_mesh(enabled);
}

/** Whether CPU mesh path is active. */
export function getUseCpuMesh(): boolean {
  return renderer?.get_use_cpu_mesh() ?? false;
}

/** Stop the render loop and release GPU resources. */
export function destroy() {
  cancelAnimationFrame(rafId);
  renderer?.free();
  renderer = null;
}

/** Whether the renderer has been initialized. */
export function isReady(): boolean {
  return renderer !== null;
}
