/**
 * Main-thread client for the mesher web worker.
 *
 * Provides a promise-based API for submitting mesh jobs and
 * automatically cancels stale jobs (latest-wins).
 */

import type {
  MesherRequest,
  MesherResponse,
  MeshJobParams,
  MeshJobResult,
} from "./mesherTypes";

export type ProgressStage = "generating" | "meshing" | "extracting";
export type ProgressCallback = (stage: ProgressStage) => void;

export class MesherClient {
  private worker: Worker;
  private nextJobId = 1;
  private currentJobId: number | null = null;

  // Init promise callbacks
  private resolveInit: ((version: string) => void) | null = null;
  private rejectInit: ((error: Error) => void) | null = null;

  // Mesh promise callbacks
  private resolveMesh: ((result: MeshJobResult) => void) | null = null;
  private rejectMesh: ((error: Error) => void) | null = null;
  private onProgress: ProgressCallback | null = null;

  constructor() {
    this.worker = new Worker(
      new URL("./mesher.worker.ts", import.meta.url),
      { type: "module" },
    );
    this.worker.addEventListener("message", this.handleMessage);
  }

  /** Initialize the worker and load WASM. Resolves with the WASM version. */
  init(): Promise<string> {
    return new Promise<string>((resolve, reject) => {
      this.resolveInit = resolve;
      this.rejectInit = reject;
      this.send({ type: "init" });
    });
  }

  /**
   * Submit a mesh job. Automatically cancels any in-flight job.
   * Resolves with the mesh result containing transferred typed arrays.
   */
  mesh(
    params: Omit<MeshJobParams, "jobId">,
    onProgress?: ProgressCallback,
  ): Promise<MeshJobResult> {
    // Cancel previous job
    if (this.currentJobId !== null) {
      this.send({ type: "cancel", jobId: this.currentJobId });
      this.rejectMesh?.(new Error("cancelled"));
      this.clearMeshCallbacks();
    }

    const jobId = this.nextJobId++;
    this.currentJobId = jobId;

    return new Promise<MeshJobResult>((resolve, reject) => {
      this.resolveMesh = resolve;
      this.rejectMesh = reject;
      this.onProgress = onProgress ?? null;
      this.send({ type: "mesh", params: { ...params, jobId } });
    });
  }

  /** Get the underlying Worker for sharing with ChunkManagerClient. */
  getWorker(): Worker {
    return this.worker;
  }

  /** Terminate the worker and release resources. */
  dispose(): void {
    this.worker.removeEventListener("message", this.handleMessage);
    this.worker.terminate();
    this.clearMeshCallbacks();
    this.resolveInit = null;
    this.rejectInit = null;
  }

  // ---------------------------------------------------------------------------
  // Private
  // ---------------------------------------------------------------------------

  private send(request: MesherRequest): void {
    this.worker.postMessage(request);
  }

  private handleMessage = (e: MessageEvent<MesherResponse>): void => {
    const msg = e.data;

    switch (msg.type) {
      case "init-done":
        this.resolveInit?.(msg.version);
        this.resolveInit = null;
        this.rejectInit = null;
        break;

      case "init-error":
        this.rejectInit?.(new Error(msg.error));
        this.resolveInit = null;
        this.rejectInit = null;
        break;

      case "mesh-done":
        if (msg.result.jobId === this.currentJobId) {
          this.resolveMesh?.(msg.result);
          this.clearMeshCallbacks();
        }
        break;

      case "mesh-error":
        if (msg.jobId === this.currentJobId) {
          this.rejectMesh?.(new Error(msg.error));
          this.clearMeshCallbacks();
        }
        break;

      case "progress":
        if (msg.jobId === this.currentJobId) {
          this.onProgress?.(msg.stage);
        }
        break;
    }
  };

  private clearMeshCallbacks(): void {
    this.resolveMesh = null;
    this.rejectMesh = null;
    this.onProgress = null;
  }
}
