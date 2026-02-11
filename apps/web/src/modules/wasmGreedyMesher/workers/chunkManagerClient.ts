/**
 * Main-thread client for the chunk manager running in the mesher web worker.
 *
 * Provides a promise-based API for multi-chunk voxel world management.
 * Shares the same Worker instance as MesherClient — messages are routed
 * by the `cm-` prefix convention.
 */

import type {
  ChunkManagerRequest,
  ChunkManagerResponse,
  ChunkMeshTransfer,
  ChunkCoord,
  FrameStats,
  ChunkDebugInfo,
  RebuildConfig,
  MemoryBudgetConfig,
} from "./chunkManagerTypes";

/** Result of a frame update. */
export type UpdateResult = {
  stats: FrameStats;
  swappedMeshes: ChunkMeshTransfer[];
  evictedCoords: ChunkCoord[];
};

/** Result of populate_dense operation. */
export type PopulateDenseResult = {
  chunksRebuilt: number;
  swappedMeshes: ChunkMeshTransfer[];
  genTime: number;
  meshTime: number;
};

export class ChunkManagerClient {
  private worker: Worker;

  // Init promise
  private resolveInit: ((voxelSize: number) => void) | null = null;
  private rejectInit: ((error: Error) => void) | null = null;

  // Update promise (at most one in-flight)
  private resolveUpdate: ((result: UpdateResult) => void) | null = null;
  private rejectUpdate: ((error: Error) => void) | null = null;

  // getVoxel pending requests
  private nextRequestId = 1;
  private voxelRequests = new Map<number, {
    resolve: (material: number) => void;
    reject: (error: Error) => void;
  }>();

  // debugInfo promise
  private resolveDebug: ((info: ChunkDebugInfo) => void) | null = null;
  private rejectDebug: ((error: Error) => void) | null = null;

  // populate promise
  private resolvePopulate: ((result: PopulateDenseResult) => void) | null = null;
  private rejectPopulate: ((error: Error) => void) | null = null;

  /**
   * Create a client that shares an existing worker.
   *
   * The worker must already have the mesher WASM module initialized
   * (send `init` via MesherClient first).
   */
  constructor(worker: Worker) {
    this.worker = worker;
    this.worker.addEventListener("message", this.handleMessage);
  }

  // =========================================================================
  // Lifecycle
  // =========================================================================

  /**
   * Initialize the chunk manager in the worker.
   * Resolves with the voxel size used by the manager.
   */
  initChunkManager(config?: RebuildConfig, budget?: MemoryBudgetConfig): Promise<number> {
    return new Promise<number>((resolve, reject) => {
      this.resolveInit = resolve;
      this.rejectInit = reject;
      this.send({ type: "cm-init", config, budget });
    });
  }

  /** Send cm-clear and remove the message listener. */
  dispose(): void {
    this.send({ type: "cm-clear" });
    this.worker.removeEventListener("message", this.handleMessage);
    this.resolveInit = null;
    this.rejectInit = null;
    this.rejectUpdate?.(new Error("disposed"));
    this.resolveUpdate = null;
    this.rejectUpdate = null;
    this.rejectPopulate?.(new Error("disposed"));
    this.resolvePopulate = null;
    this.rejectPopulate = null;
    this.resolveDebug = null;
    this.rejectDebug = null;
    for (const req of this.voxelRequests.values()) {
      req.reject(new Error("disposed"));
    }
    this.voxelRequests.clear();
  }

  // =========================================================================
  // Voxel Editing (fire-and-forget)
  // =========================================================================

  /** Set a voxel at world-space coordinates. */
  setVoxel(wx: number, wy: number, wz: number, material: number): void {
    this.send({ type: "cm-set-voxel", wx, wy, wz, material });
  }

  /** Set a voxel at integer voxel-space coordinates. */
  setVoxelAt(vx: number, vy: number, vz: number, material: number): void {
    this.send({ type: "cm-set-voxel-at", vx, vy, vz, material });
  }

  /** Set multiple voxels in one message. Flat array: [wx, wy, wz, material, ...]. */
  setVoxelsBatch(edits: Float32Array): void {
    this.send({ type: "cm-set-voxels-batch", edits });
  }

  /** Get the material at a world-space coordinate. */
  getVoxel(wx: number, wy: number, wz: number): Promise<number> {
    const requestId = this.nextRequestId++;
    return new Promise<number>((resolve, reject) => {
      this.voxelRequests.set(requestId, { resolve, reject });
      this.send({ type: "cm-get-voxel", wx, wy, wz, requestId });
    });
  }

  // =========================================================================
  // Frame Update
  // =========================================================================

  /**
   * Run one frame update cycle (rebuild + swap + evict).
   *
   * At most one update is in-flight at a time. If called while a previous
   * update is pending, the previous promise is rejected with "superseded".
   */
  update(camX: number, camY: number, camZ: number): Promise<UpdateResult> {
    // Supersede any in-flight update
    if (this.resolveUpdate) {
      this.rejectUpdate?.(new Error("superseded"));
      this.resolveUpdate = null;
      this.rejectUpdate = null;
    }

    return new Promise<UpdateResult>((resolve, reject) => {
      this.resolveUpdate = resolve;
      this.rejectUpdate = reject;
      this.send({ type: "cm-update", camX, camY, camZ });
    });
  }

  // =========================================================================
  // Generate & Populate (one-shot large grid)
  // =========================================================================

  /**
   * Generate a voxel grid and populate chunks in one worker round-trip.
   *
   * This is optimized for large grids (>62) where setting voxels individually
   * would be too slow. The worker generates the voxel pattern in JS, passes
   * it to WASM populate_dense, rebuilds all dirty chunks, and returns meshes.
   */
  generateAndPopulate(opts: {
    gridSize: number;
    voxelSize: number;
    pattern: "solid" | "checkerboard" | "sphere" | "noise" | "perlin" | "simplex";
    simplexScale?: number;
    simplexOctaves?: number;
    simplexThreshold?: number;
  }): Promise<PopulateDenseResult> {
    // Reject any in-flight populate
    if (this.resolvePopulate) {
      this.rejectPopulate?.(new Error("superseded"));
      this.resolvePopulate = null;
      this.rejectPopulate = null;
    }

    return new Promise<PopulateDenseResult>((resolve, reject) => {
      this.resolvePopulate = resolve;
      this.rejectPopulate = reject;
      this.send({
        type: "cm-generate-and-populate",
        gridSize: opts.gridSize,
        voxelSize: opts.voxelSize,
        pattern: opts.pattern,
        simplexScale: opts.simplexScale,
        simplexOctaves: opts.simplexOctaves,
        simplexThreshold: opts.simplexThreshold,
      });
    });
  }

  // =========================================================================
  // Budget & Management (fire-and-forget)
  // =========================================================================

  /** Update the memory budget configuration. */
  setBudget(budget: MemoryBudgetConfig): void {
    this.send({ type: "cm-set-budget", budget });
  }

  /** Mark a chunk as recently accessed (resets LRU age). */
  touchChunk(cx: number, cy: number, cz: number): void {
    this.send({ type: "cm-touch-chunk", cx, cy, cz });
  }

  /** Remove a specific chunk. */
  removeChunk(cx: number, cy: number, cz: number): void {
    this.send({ type: "cm-remove-chunk", cx, cy, cz });
  }

  /** Remove all chunks. */
  clear(): void {
    this.send({ type: "cm-clear" });
  }

  // =========================================================================
  // Debug
  // =========================================================================

  /** Get debug info from the chunk manager. */
  debugInfo(): Promise<ChunkDebugInfo> {
    return new Promise<ChunkDebugInfo>((resolve, reject) => {
      this.resolveDebug = resolve;
      this.rejectDebug = reject;
      this.send({ type: "cm-debug-info" });
    });
  }

  // =========================================================================
  // Private
  // =========================================================================

  private send(request: ChunkManagerRequest): void {
    this.worker.postMessage(request);
  }

  private handleMessage = (e: MessageEvent): void => {
    const msg = e.data as ChunkManagerResponse;

    // Only handle cm-* messages
    if (typeof msg?.type !== "string" || !msg.type.startsWith("cm-")) return;

    switch (msg.type) {
      case "cm-init-done":
        this.resolveInit?.(msg.voxelSize);
        this.resolveInit = null;
        this.rejectInit = null;
        break;

      case "cm-init-error":
        this.rejectInit?.(new Error(msg.error));
        this.resolveInit = null;
        this.rejectInit = null;
        break;

      case "cm-update-done":
        this.resolveUpdate?.({
          stats: msg.stats,
          swappedMeshes: msg.swappedMeshes,
          evictedCoords: msg.evictedCoords,
        });
        this.resolveUpdate = null;
        this.rejectUpdate = null;
        break;

      case "cm-voxel-result": {
        const req = this.voxelRequests.get(msg.requestId);
        if (req) {
          req.resolve(msg.material);
          this.voxelRequests.delete(msg.requestId);
        }
        break;
      }

      case "cm-debug-info-result":
        this.resolveDebug?.(msg.info);
        this.resolveDebug = null;
        this.rejectDebug = null;
        break;

      case "cm-populate-done":
        this.resolvePopulate?.({
          chunksRebuilt: msg.chunksRebuilt,
          swappedMeshes: msg.swappedMeshes,
          genTime: msg.genTime,
          meshTime: msg.meshTime,
        });
        this.resolvePopulate = null;
        this.rejectPopulate = null;
        break;

      case "cm-error":
        // Route generic errors to any pending promise
        if (this.rejectPopulate) {
          this.rejectPopulate(new Error(msg.error));
          this.resolvePopulate = null;
          this.rejectPopulate = null;
        } else if (this.rejectUpdate) {
          this.rejectUpdate(new Error(msg.error));
          this.resolveUpdate = null;
          this.rejectUpdate = null;
        } else if (this.rejectDebug) {
          this.rejectDebug(new Error(msg.error));
          this.resolveDebug = null;
          this.rejectDebug = null;
        }
        break;
    }
  };
}
