import { writable } from "svelte/store";

export interface RendererStats {
  frame: number;
  residentCount: number;
  renderMode: number;
  totalVoxels: number;
  meshVerts: number;
  meshIndices: number;
  meshQuads: number;
  cameraPos: [number, number, number];
  cameraDir: [number, number, number];
  // Layer 1 additions
  viewportWidth: number;
  viewportHeight: number;
  cameraFov: number;
  cameraNear: number;
  cameraFar: number;
  cameraAspect: number;
  freeSlots: number;
  hasWireframe: boolean;
  backfaceCulling: boolean;
  depthPrepassEnabled: boolean;
}

export const rendererStatsStore = writable<RendererStats | null>(null);
