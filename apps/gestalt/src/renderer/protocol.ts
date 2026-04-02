/**
 * Renderer Protocol
 *
 * Shared type definitions for the rendering pipeline.
 * ADR-0014: All GPU work runs on the main thread. No worker, no SharedArrayBuffer.
 * This file retains the RenderMode enum and command constants used by the controller.
 */

// ---------------------------------------------------------------------------
// Render modes
// ---------------------------------------------------------------------------

export const RenderMode = {
  // ── Working views ──────────────────────────────────────────────────────
  Solid:          0x00,  // Flat directional light, no GI. Default working mode.
  GI:             0x01,  // Full cascade GI (beauty mode). Active when cascades are wired.
  Wireframe:      0x02,  // Edges only, transparent faces.
  SolidWireframe: 0x03,  // Solid shading + wireframe edge overlay.
  Normals:        0x04,  // World-space normal → RGB.
  Matcap:         0x05,  // Spherical environment material for quick surface reads.

  // ── GPU debug views ────────────────────────────────────────────────────
  Depth:          0x10,  // Grayscale depth buffer visualization.
  HiZMip:         0x11,  // Selected mip level of the Hi-Z pyramid.
  ChunkState:     0x12,  // Color chunks by lifecycle state (clean/dirty/empty).
  Occupancy:      0x13,  // Bricklet occupancy density heatmap.
} as const;

export type RenderMode = (typeof RenderMode)[keyof typeof RenderMode];
