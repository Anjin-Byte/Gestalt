/**
 * Message protocol types for the mesher web worker.
 *
 * Defines the request/response contract between the main thread
 * (MesherClient) and the worker (mesher.worker.ts).
 */

export type VoxelPattern = "solid" | "checkerboard" | "sphere" | "noise";
export type DebugColorMode = "none" | "face-direction" | "quad-size";

/** Parameters for a mesh job sent to the worker. */
export type MeshJobParams = {
  jobId: number;
  gridSize: number;
  voxelSize: number;
  pattern: VoxelPattern;
  debugMode: boolean;
  debugColorMode: DebugColorMode;
  debugWireframe: boolean;
};

/** Statistics returned with a mesh result. */
export type MeshJobStats = {
  genTime: number;
  meshTime: number;
  triCount: number;
  vtxCount: number;
  quadCount?: number;
  efficiency?: number;
  reduction?: number;
  dirQuadCounts?: number[];
  dirFaceCounts?: number[];
};

/** Complete mesh result with typed arrays ready for transfer. */
export type MeshJobResult = {
  jobId: number;
  positions: Float32Array;
  normals: Float32Array;
  indices: Uint32Array;
  wirePositions?: Float32Array;
  faceColors?: Float32Array;
  sizeColors?: Float32Array;
  stats: MeshJobStats;
};

/** Messages sent from main thread to worker. */
export type MesherRequest =
  | { type: "init" }
  | { type: "mesh"; params: MeshJobParams }
  | { type: "cancel"; jobId: number };

/** Messages sent from worker to main thread. */
export type MesherResponse =
  | { type: "init-done"; version: string }
  | { type: "init-error"; error: string }
  | { type: "mesh-done"; result: MeshJobResult }
  | { type: "mesh-error"; jobId: number; error: string }
  | { type: "progress"; jobId: number; stage: "generating" | "meshing" | "extracting" };
