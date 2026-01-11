# Voxelizer JS Adapter

Thin JavaScript/TypeScript adapter for the WASM voxelizer. This wraps WASM
initialization, input normalization, voxelization calls, and CPU-side helpers
like sparse-to-positions expansion and brick paging.

## Install (workspace)

This package is part of the repo workspace. Import it by name:

```ts
import { VoxelizerAdapter } from "@gestalt/voxelizer-js";
```

## Usage

```ts
import { VoxelizerAdapter } from "@gestalt/voxelizer-js";

const voxelizer = await VoxelizerAdapter.create({
  loadWasm: async () => await import("../wasm/wasm_voxelizer/wasm_voxelizer.js"),
  logEnabled: true
});

const positions = new Float32Array([...]);
const indices = new Uint32Array([...]);

const { grid, origin, voxelSize } = voxelizer.resolveGrid({
  positions,
  gridDim: 256,
  voxelSize: 0.1,
  fitBounds: true
});

const sparse = await voxelizer.voxelizeSparse({
  positions,
  indices,
  grid,
  epsilon: 1e-3
});

const points = voxelizer.expandSparseToPositions(sparse, origin, voxelSize);
```

## API overview

- `VoxelizerAdapter.create({ loadWasm, wasmModule, logEnabled })`
- `setLogEnabled(enabled)`
- `resolveGrid({ positions, gridDim, voxelSize, origin?, fitBounds? })`
- `voxelizeSparse({ positions, indices, grid, epsilon? })`
- `voxelizeSparseChunked({ positions, indices, grid, epsilon?, chunkSize, compact })`
- `voxelizePositions({ positions, indices, grid, epsilon?, maxPositions })`
- `voxelizePositionsChunked({ positions, indices, grid, epsilon?, chunkSize, maxPositions })`
- `expandSparseToPositions(sparse, origin, voxelSize)`
- `flattenBricksFromChunks(chunks)`
- `pageBricks(bricks, { page, bricksPerPage })`
- `buildPositionsForBricks(bricks, voxelSize, origin)`

## Notes

- This adapter intentionally does **not** depend on three.js or any renderer.
- The caller is responsible for rendering and visualization.
- Paging is a data-level selection of bricks (no rendering logic).
