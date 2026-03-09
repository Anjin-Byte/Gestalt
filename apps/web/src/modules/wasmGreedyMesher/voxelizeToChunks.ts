/**
 * Integration bridge: OBJ → GPU voxelize → ChunkManager ingest → greedy mesh.
 *
 * Orchestrates the ADR-0009 pipeline:
 * 1. GPU voxelizer produces CompactVoxel data (vx, vy, vz, material)
 * 2. ChunkManager ingests compact voxels into chunk system
 * 3. Frame update triggers greedy meshing → returns mesh outputs
 */

import type { ModuleOutput } from "../types";
import type { ChunkManagerClient } from "./workers/chunkManagerClient";
import type { ChunkMeshTransfer } from "./workers/chunkManagerTypes";
import { CS, generateBoxWireframe } from "./helpers";

/** Options for voxelizeToChunks. */
export type VoxelizeToChunksOpts = {
  /** Flat Float32Array of vertex positions (x, y, z triples). */
  positions: Float32Array;
  /** Uint32Array of triangle indices. */
  indices: Uint32Array;
  /** Per-triangle u16 material IDs (from buildMaterialTable). */
  materialTable: Uint16Array;
  /** World-space origin (x, y, z). */
  origin: [number, number, number];
  /** Voxel size in world units. */
  voxelSize: number;
  /** Grid dimensions [x, y, z]. */
  dims: [number, number, number];
  /** Epsilon for voxel-triangle intersection. */
  epsilon?: number;
  /** Show chunk boundary wireframes. */
  debugChunkBounds?: boolean;
  /** Color mode for debug visualization. */
  colorMode?: "none" | "material" | "chunk" | "face-direction" | "quad-size";
  /** Show quad boundary wireframe overlay. */
  debugWireframe?: boolean;
};

/**
 * Pack a Uint16Array of material IDs into Uint32Array (two per word).
 *
 * Element `i` goes into word `i >> 1`, low 16 bits for even indices,
 * high 16 bits for odd indices. This matches the GPU shader's unpacking:
 * `word_idx = tri >> 1; shift = (tri & 1) << 4`.
 */
export const packMaterialTable = (table: Uint16Array): Uint32Array => {
  const wordCount = (table.length + 1) >>> 1;
  const packed = new Uint32Array(wordCount);
  for (let i = 0; i < table.length; i++) {
    const wordIdx = i >>> 1;
    const shift = (i & 1) << 4;
    packed[wordIdx] |= (table[i] & 0xffff) << shift;
  }
  return packed;
};

// === Debug Color Generation ===

/** Deterministic color palette for material IDs. Golden-ratio hue spacing for perceptual distinctness. */
const materialColor = (materialId: number): [number, number, number] => {
  if (materialId <= 1) return [0.7, 0.7, 0.7]; // default/unset → grey
  const hue = ((materialId * 0.618033988749895) % 1.0);
  return hslToRgb(hue, 0.7, 0.55);
};

/** Deterministic color for a chunk coordinate. */
const chunkColor = (cx: number, cy: number, cz: number): [number, number, number] => {
  const hash = ((cx * 73856093) ^ (cy * 19349663) ^ (cz * 83492791)) >>> 0;
  const hue = (hash % 360) / 360;
  return hslToRgb(hue, 0.65, 0.5);
};

/** Face-direction colors matching Rust debug.rs conventions. */
const FACE_DIR_COLORS: [number, number, number][] = [
  [0.35, 0.85, 0.35], // +Y (top) — green
  [0.85, 0.35, 0.35], // -Y (bottom) — red
  [0.35, 0.35, 0.85], // +X — blue
  [0.85, 0.85, 0.35], // -X — yellow
  [0.85, 0.35, 0.85], // +Z — magenta
  [0.35, 0.85, 0.85], // -Z — cyan
];

/** Map a vertex normal to one of 6 axis-aligned directions. */
const normalToFaceDir = (nx: number, ny: number, nz: number): number => {
  const ax = Math.abs(nx), ay = Math.abs(ny), az = Math.abs(nz);
  if (ay >= ax && ay >= az) return ny >= 0 ? 0 : 1;
  if (ax >= ay && ax >= az) return nx >= 0 ? 2 : 3;
  return nz >= 0 ? 4 : 5;
};

const hslToRgb = (h: number, s: number, l: number): [number, number, number] => {
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h * 6) % 2) - 1));
  const m = l - c / 2;
  let r = 0, g = 0, b = 0;
  const sector = Math.floor(h * 6) % 6;
  if (sector === 0) { r = c; g = x; }
  else if (sector === 1) { r = x; g = c; }
  else if (sector === 2) { g = c; b = x; }
  else if (sector === 3) { g = x; b = c; }
  else if (sector === 4) { r = x; b = c; }
  else { r = c; b = x; }
  return [r + m, g + m, b + m];
};

/**
 * Quad-size heatmap color (matches Rust debug.rs log scale).
 * Small quads → red, large quads → green.
 */
const quadSizeColor = (width: number, height: number): [number, number, number] => {
  const area = width * height;
  const maxLog = Math.log(62 * 62);
  const t = Math.max(0, Math.min(1, Math.log(Math.max(1, area)) / maxLog));
  return [1 - t, t, 0.1];
};

/** Generate per-vertex colors for a chunk mesh based on color mode. */
const generateColors = (
  mesh: ChunkMeshTransfer,
  mode: "material" | "chunk" | "face-direction" | "quad-size",
): Float32Array => {
  const vertCount = mesh.positions.length / 3;
  const colors = new Float32Array(vertCount * 3);

  if (mode === "material") {
    for (let i = 0; i < vertCount; i++) {
      const matId = mesh.materialIds[i] ?? 0;
      const [r, g, b] = materialColor(matId);
      colors[i * 3] = r;
      colors[i * 3 + 1] = g;
      colors[i * 3 + 2] = b;
    }
  } else if (mode === "chunk") {
    const [r, g, b] = chunkColor(mesh.coord.x, mesh.coord.y, mesh.coord.z);
    for (let i = 0; i < vertCount; i++) {
      colors[i * 3] = r;
      colors[i * 3 + 1] = g;
      colors[i * 3 + 2] = b;
    }
  } else if (mode === "face-direction") {
    for (let i = 0; i < vertCount; i++) {
      const nx = mesh.normals[i * 3];
      const ny = mesh.normals[i * 3 + 1];
      const nz = mesh.normals[i * 3 + 2];
      const dir = normalToFaceDir(nx, ny, nz);
      const [r, g, b] = FACE_DIR_COLORS[dir];
      colors[i * 3] = r;
      colors[i * 3 + 1] = g;
      colors[i * 3 + 2] = b;
    }
  } else if (mode === "quad-size") {
    // Each quad = 4 vertices. UVs encode quad dimensions:
    // corners are (0,0), (w,0), (w,h), (0,h).
    // Max UV across each group of 4 vertices gives width and height.
    for (let q = 0; q < vertCount; q += 4) {
      let maxU = 0, maxV = 0;
      for (let v = 0; v < 4 && q + v < vertCount; v++) {
        const u = Math.abs(mesh.uvs[(q + v) * 2] ?? 0);
        const vv = Math.abs(mesh.uvs[(q + v) * 2 + 1] ?? 0);
        if (u > maxU) maxU = u;
        if (vv > maxV) maxV = vv;
      }
      const [r, g, b] = quadSizeColor(Math.max(1, Math.round(maxU)), Math.max(1, Math.round(maxV)));
      for (let v = 0; v < 4 && q + v < vertCount; v++) {
        colors[(q + v) * 3] = r;
        colors[(q + v) * 3 + 1] = g;
        colors[(q + v) * 3 + 2] = b;
      }
    }
  }

  return colors;
};

/**
 * Generate quad boundary wireframe lines from mesh data.
 * Each quad = 4 sequential vertices → 4 edges.
 */
const generateWireframe = (mesh: ChunkMeshTransfer): Float32Array => {
  const vertCount = mesh.positions.length / 3;
  const quadCount = Math.floor(vertCount / 4);
  // 4 edges per quad, 2 endpoints per edge, 3 floats per endpoint
  const lines = new Float32Array(quadCount * 24);
  let idx = 0;

  for (let q = 0; q < quadCount; q++) {
    const base = q * 4;
    for (let e = 0; e < 4; e++) {
      const a = base + e;
      const b = base + ((e + 1) % 4);
      lines[idx++] = mesh.positions[a * 3];
      lines[idx++] = mesh.positions[a * 3 + 1];
      lines[idx++] = mesh.positions[a * 3 + 2];
      lines[idx++] = mesh.positions[b * 3];
      lines[idx++] = mesh.positions[b * 3 + 1];
      lines[idx++] = mesh.positions[b * 3 + 2];
    }
  }

  return lines;
};

/**
 * Compute the global voxel-space origin from world origin and voxel size.
 * g_origin = floor(origin / voxelSize)
 */
const computeGOrigin = (
  origin: [number, number, number],
  voxelSize: number
): [number, number, number] => [
  Math.floor(origin[0] / voxelSize),
  Math.floor(origin[1] / voxelSize),
  Math.floor(origin[2] / voxelSize),
];

/**
 * Run the full OBJ → voxelize → chunk → mesh pipeline.
 *
 * Requires both a WasmVoxelizer instance (for GPU compact) and a
 * ChunkManagerClient (for ingestion + meshing).
 *
 * @param voxelizer - WasmVoxelizer instance (from wasm_voxelizer)
 * @param chunkManagerClient - Initialized ChunkManagerClient
 * @param opts - Pipeline options
 * @returns ModuleOutput array with mesh and optional debug outputs
 */
export const voxelizeToChunks = async (
  voxelizer: {
    voxelize_compact_voxels(
      positions: Float32Array,
      indices: Uint32Array,
      materialTable: Uint32Array,
      origin: Float32Array,
      voxelSize: number,
      dims: Uint32Array,
      epsilon: number,
      gOriginX: number,
      gOriginY: number,
      gOriginZ: number,
    ): Promise<{ voxels: Int32Array; count: number }>;
  },
  chunkManagerClient: ChunkManagerClient,
  opts: VoxelizeToChunksOpts
): Promise<ModuleOutput[]> => {
  const epsilon = opts.epsilon ?? 1e-4;
  const gOrigin = computeGOrigin(opts.origin, opts.voxelSize);
  const packedTable = packMaterialTable(opts.materialTable);

  // Step 1: GPU voxelize + compact
  const originArr = new Float32Array(opts.origin);
  const dimsArr = new Uint32Array(opts.dims);

  const compactResult = await voxelizer.voxelize_compact_voxels(
    opts.positions,
    opts.indices,
    packedTable,
    originArr,
    opts.voxelSize,
    dimsArr,
    epsilon,
    gOrigin[0],
    gOrigin[1],
    gOrigin[2],
  );

  if (compactResult.count === 0) {
    return [];
  }

  // Step 2: Ingest into ChunkManager
  await chunkManagerClient.ingestCompactVoxels(compactResult.voxels);

  // Step 3: Rebuild all dirty chunks and get meshes
  // Uses rebuildAllDirty instead of update() because std::time::Instant
  // is not available on wasm32-unknown-unknown.
  const rebuildResult = await chunkManagerClient.rebuildAllDirty();

  // Step 4: Convert to ModuleOutput
  const outputs: ModuleOutput[] = [];
  const colorMode = opts.colorMode ?? "none";

  for (const mesh of rebuildResult.swappedMeshes) {
    const meshDesc: { positions: Float32Array; indices: Uint32Array; normals: Float32Array; colors?: Float32Array } = {
      positions: mesh.positions,
      indices: mesh.indices,
      normals: mesh.normals,
    };

    if (colorMode !== "none") {
      meshDesc.colors = generateColors(mesh, colorMode);
    }

    outputs.push({
      kind: "mesh",
      mesh: meshDesc,
      label: `Chunk (${mesh.coord.x},${mesh.coord.y},${mesh.coord.z})`,
    });
  }

  // Optional: quad boundary wireframe
  if (opts.debugWireframe) {
    for (const mesh of rebuildResult.swappedMeshes) {
      const wirePositions = generateWireframe(mesh);
      if (wirePositions.length > 0) {
        outputs.push({
          kind: "lines",
          lines: { positions: wirePositions, color: [1.0, 1.0, 0.0] },
          label: `Wireframe (${mesh.coord.x},${mesh.coord.y},${mesh.coord.z})`,
        });
      }
    }
  }

  // Optional: debug chunk bounds
  if (opts.debugChunkBounds) {
    for (const mesh of rebuildResult.swappedMeshes) {
      const offsetX = mesh.coord.x * CS * opts.voxelSize;
      const offsetY = mesh.coord.y * CS * opts.voxelSize;
      const offsetZ = mesh.coord.z * CS * opts.voxelSize;
      const chunkExtent = CS * opts.voxelSize;

      const boundsPositions = generateBoxWireframe(
        offsetX,
        offsetY,
        offsetZ,
        offsetX + chunkExtent,
        offsetY + chunkExtent,
        offsetZ + chunkExtent,
      );
      outputs.push({
        kind: "lines",
        lines: { positions: boundsPositions, color: [0.0, 1.0, 1.0] },
        label: `Bounds (${mesh.coord.x},${mesh.coord.y},${mesh.coord.z})`,
      });
    }
  }

  return outputs;
};
