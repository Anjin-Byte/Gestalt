export type Vec3 = [number, number, number];

export type VoxelGridSpec = {
  origin: Vec3;
  voxelSize: number;
  dims: [number, number, number];
};

export type SparseVoxelOutput = {
  occupancy: Uint32Array;
  brick_dim: number;
  brick_origins: Uint32Array;
  fallback_used?: boolean;
  debug_flags?: Uint32Array;
  debug_workgroups?: number;
  debug_tested?: number;
  debug_hits?: number;
};

export type SparseVoxelChunk = SparseVoxelOutput;

export type PositionsOutput = {
  positions: Float32Array;
  brick_dim: number;
  brick_count: number;
  fallback_used?: boolean;
  debug_workgroups?: number;
  debug_tested?: number;
  debug_hits?: number;
};

export type PositionsChunk = {
  positions: Float32Array;
  count: number;
  brick_dim: number;
  brick_count: number;
};

export type VoxelizeRequest = {
  positions: Float32Array;
  indices: Uint32Array;
  grid: VoxelGridSpec;
  epsilon?: number;
};

export type VoxelizeChunkedRequest = VoxelizeRequest & {
  chunkSize: number;
  compact: boolean;
};

export type VoxelizePositionsRequest = VoxelizeRequest & {
  maxPositions: number;
};

export type VoxelizePositionsChunkedRequest = VoxelizeRequest & {
  chunkSize: number;
  maxPositions: number;
};

export type BrickRef = {
  occupancy: Uint32Array;
  wordOffset: number;
  origin: [number, number, number];
  brickDim: number;
};

export type PageResult = {
  page: number;
  totalPages: number;
  bricksPerPage: number;
  bricks: BrickRef[];
};

type WasmVoxelizerModule = {
  default?: () => Promise<unknown>;
  set_log_enabled?: (enabled: boolean) => void;
  WasmVoxelizer?: {
    new?: () => Promise<WasmVoxelizerInstance>;
  };
};

type WasmVoxelizerInstance = {
  voxelize_triangles: (
    positions: Float32Array,
    indices: Uint32Array,
    origin: Float32Array,
    voxelSize: number,
    dims: Uint32Array,
    epsilon: number
  ) => Promise<SparseVoxelOutput>;
  voxelize_triangles_positions: (
    positions: Float32Array,
    indices: Uint32Array,
    origin: Float32Array,
    voxelSize: number,
    dims: Uint32Array,
    epsilon: number,
    maxPositions: number
  ) => Promise<PositionsOutput>;
  voxelize_triangles_positions_chunked: (
    positions: Float32Array,
    indices: Uint32Array,
    origin: Float32Array,
    voxelSize: number,
    dims: Uint32Array,
    epsilon: number,
    chunkSize: number,
    maxPositions: number
  ) => Promise<PositionsChunk[]>;
  voxelize_triangles_chunked: (
    positions: Float32Array,
    indices: Uint32Array,
    origin: Float32Array,
    voxelSize: number,
    dims: Uint32Array,
    epsilon: number,
    chunkSize: number,
    compact: boolean
  ) => Promise<SparseVoxelChunk[]>;
};

export type VoxelizerAdapterOptions = {
  loadWasm?: () => Promise<WasmVoxelizerModule>;
  wasmModule?: WasmVoxelizerModule;
  logEnabled?: boolean;
};

const clamp = (value: number, min: number, max: number) =>
  Math.min(Math.max(value, min), max);

const computeBounds = (positions: Float32Array) => {
  let minX = Number.POSITIVE_INFINITY;
  let minY = Number.POSITIVE_INFINITY;
  let minZ = Number.POSITIVE_INFINITY;
  let maxX = Number.NEGATIVE_INFINITY;
  let maxY = Number.NEGATIVE_INFINITY;
  let maxZ = Number.NEGATIVE_INFINITY;

  for (let i = 0; i < positions.length; i += 3) {
    const x = positions[i];
    const y = positions[i + 1];
    const z = positions[i + 2];
    if (x < minX) minX = x;
    if (y < minY) minY = y;
    if (z < minZ) minZ = z;
    if (x > maxX) maxX = x;
    if (y > maxY) maxY = y;
    if (z > maxZ) maxZ = z;
  }

  if (!Number.isFinite(minX)) {
    return null;
  }

  return {
    min: [minX, minY, minZ] as Vec3,
    max: [maxX, maxY, maxZ] as Vec3
  };
};

export class VoxelizerAdapter {
  private constructor(
    private wasmModule: WasmVoxelizerModule,
    private voxelizer: WasmVoxelizerInstance
  ) {}

  static async create(options: VoxelizerAdapterOptions): Promise<VoxelizerAdapter> {
    const wasmModule =
      options.wasmModule ??
      (options.loadWasm ? await options.loadWasm() : null);
    if (!wasmModule) {
      throw new Error("VoxelizerAdapter requires wasmModule or loadWasm");
    }
    if (wasmModule.default) {
      await wasmModule.default();
    }
    const voxelizer = (await wasmModule.WasmVoxelizer?.new?.()) ?? null;
    if (!voxelizer) {
      throw new Error("WASM voxelizer exports missing");
    }
    const adapter = new VoxelizerAdapter(wasmModule, voxelizer);
    if (options.logEnabled !== undefined) {
      adapter.setLogEnabled(options.logEnabled);
    }
    return adapter;
  }

  setLogEnabled(enabled: boolean): void {
    this.wasmModule.set_log_enabled?.(enabled);
  }

  resolveGrid(options: {
    positions: Float32Array;
    gridDim: number;
    voxelSize: number;
    origin?: Vec3;
    fitBounds?: boolean;
  }): { grid: VoxelGridSpec; voxelSize: number; origin: Vec3 } {
    const gridDim = clamp(Math.floor(options.gridDim), 1, 4096);
    let voxelSize = options.voxelSize;
    let origin: Vec3 =
      options.origin ?? [
        -gridDim * voxelSize * 0.5,
        -gridDim * voxelSize * 0.5,
        -gridDim * voxelSize * 0.5
      ];

    if (options.fitBounds) {
      const bounds = computeBounds(options.positions);
      if (bounds) {
        const size = [
          bounds.max[0] - bounds.min[0],
          bounds.max[1] - bounds.min[1],
          bounds.max[2] - bounds.min[2]
        ];
        const extent = Math.max(size[0], size[1], size[2]) || 1;
        voxelSize = extent / gridDim;
        const half = extent * 0.5;
        const center: Vec3 = [
          (bounds.min[0] + bounds.max[0]) * 0.5,
          (bounds.min[1] + bounds.max[1]) * 0.5,
          (bounds.min[2] + bounds.max[2]) * 0.5
        ];
        origin = [center[0] - half, center[1] - half, center[2] - half];
      }
    }

    return {
      grid: {
        origin,
        voxelSize,
        dims: [gridDim, gridDim, gridDim]
      },
      voxelSize,
      origin
    };
  }

  async voxelizeSparse(request: VoxelizeRequest): Promise<SparseVoxelOutput> {
    const { positions, indices, grid } = request;
    const epsilon = request.epsilon ?? 1e-3;
    return this.voxelizer.voxelize_triangles(
      positions,
      indices,
      new Float32Array(grid.origin),
      grid.voxelSize,
      new Uint32Array(grid.dims),
      epsilon
    );
  }

  async voxelizeSparseChunked(
    request: VoxelizeChunkedRequest
  ): Promise<SparseVoxelChunk[]> {
    const { positions, indices, grid } = request;
    const epsilon = request.epsilon ?? 1e-3;
    return this.voxelizer.voxelize_triangles_chunked(
      positions,
      indices,
      new Float32Array(grid.origin),
      grid.voxelSize,
      new Uint32Array(grid.dims),
      epsilon,
      request.chunkSize,
      request.compact
    );
  }

  async voxelizePositions(request: VoxelizePositionsRequest): Promise<PositionsOutput> {
    const { positions, indices, grid } = request;
    const epsilon = request.epsilon ?? 1e-3;
    return this.voxelizer.voxelize_triangles_positions(
      positions,
      indices,
      new Float32Array(grid.origin),
      grid.voxelSize,
      new Uint32Array(grid.dims),
      epsilon,
      request.maxPositions
    );
  }

  async voxelizePositionsChunked(
    request: VoxelizePositionsChunkedRequest
  ): Promise<PositionsChunk[]> {
    const { positions, indices, grid } = request;
    const epsilon = request.epsilon ?? 1e-3;
    return this.voxelizer.voxelize_triangles_positions_chunked(
      positions,
      indices,
      new Float32Array(grid.origin),
      grid.voxelSize,
      new Uint32Array(grid.dims),
      epsilon,
      request.chunkSize,
      request.maxPositions
    );
  }

  expandSparseToPositions(
    output: SparseVoxelOutput,
    origin: Vec3,
    voxelSize: number
  ): Float32Array {
    return this.buildPositionsForBricks(
      this.flattenBricksFromChunks([output]),
      voxelSize,
      origin
    );
  }

  flattenBricksFromChunks(chunks: SparseVoxelChunk[]): BrickRef[] {
    const bricks: BrickRef[] = [];
    for (const chunk of chunks) {
      const brickCount = Math.floor(chunk.brick_origins.length / 3);
      const wordsPerBrick = Math.ceil(
        (chunk.brick_dim * chunk.brick_dim * chunk.brick_dim) / 32
      );
      for (let i = 0; i < brickCount; i += 1) {
        const base = i * 3;
        bricks.push({
          occupancy: chunk.occupancy,
          wordOffset: i * wordsPerBrick,
          origin: [
            chunk.brick_origins[base],
            chunk.brick_origins[base + 1],
            chunk.brick_origins[base + 2]
          ],
          brickDim: chunk.brick_dim
        });
      }
    }
    return bricks;
  }

  pageBricks(bricks: BrickRef[], options: { page: number; bricksPerPage: number }): PageResult {
    const totalBricks = bricks.length;
    const bricksPerPage = options.bricksPerPage === 0 ? totalBricks || 1 : options.bricksPerPage;
    const totalPages = totalBricks > 0 ? Math.ceil(totalBricks / bricksPerPage) : 0;
    const page = totalPages > 0 ? clamp(options.page, 0, totalPages - 1) : 0;
    const start = page * bricksPerPage;
    const end = Math.min(start + bricksPerPage, totalBricks);
    return {
      page,
      totalPages,
      bricksPerPage,
      bricks: bricks.slice(start, end)
    };
  }

  buildPositionsForBricks(
    bricks: BrickRef[],
    voxelSize: number,
    origin: Vec3
  ): Float32Array {
    const positions: number[] = [];
    for (const brick of bricks) {
      this.appendBrickPositions(
        positions,
        brick.occupancy,
        brick.wordOffset,
        brick.origin,
        brick.brickDim,
        voxelSize,
        origin
      );
    }
    return new Float32Array(positions);
  }

  private appendBrickPositions(
    positions: number[],
    occupancy: Uint32Array,
    wordOffset: number,
    brickOrigin: Vec3,
    brickDim: number,
    voxelSize: number,
    origin: Vec3
  ) {
    const brickVoxels = brickDim * brickDim * brickDim;
    const wordsPerBrick = Math.ceil(brickVoxels / 32);
    for (let wordIndex = 0; wordIndex < wordsPerBrick; wordIndex += 1) {
      const word = occupancy[wordOffset + wordIndex];
      if (!word) {
        continue;
      }
      for (let bit = 0; bit < 32; bit += 1) {
        if ((word & (1 << bit)) === 0) {
          continue;
        }
        const local = wordIndex * 32 + bit;
        if (local >= brickVoxels) {
          continue;
        }
        const lx = local % brickDim;
        const ly = Math.floor(local / brickDim) % brickDim;
        const lz = Math.floor(local / (brickDim * brickDim));
        const gx = brickOrigin[0] + lx;
        const gy = brickOrigin[1] + ly;
        const gz = brickOrigin[2] + lz;
        positions.push(
          origin[0] + (gx + 0.5) * voxelSize,
          origin[1] + (gy + 0.5) * voxelSize,
          origin[2] + (gz + 0.5) * voxelSize
        );
      }
    }
  }
}
