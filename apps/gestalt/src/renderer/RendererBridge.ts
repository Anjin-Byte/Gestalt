/**
 * RendererBridge
 *
 * Main-thread interface to the renderer worker.
 *
 * Responsibilities:
 *  - Owns the Worker instance lifecycle (create / terminate).
 *  - Provides typed builder methods that encode commands into a packed
 *    ArrayBuffer per the binary protocol defined in protocol.ts.
 *  - Transfers the command buffer to the worker via postMessage (zero-copy).
 *  - Exposes the SharedArrayBuffer handles for the ring buffer and snapshot
 *    so that Svelte stores can read them directly on the main thread.
 *
 * The bridge does NOT call WASM functions directly. All WASM interaction
 * goes through the worker.
 *
 * Usage:
 *   const bridge = await RendererBridge.create();
 *   bridge.resizeViewport(1280, 720).setCamera(...).flush();
 *   bridge.terminate();
 *
 * See: docs/architecture/wasm-boundary-protocol.md
 */

import {
  RendererOpcode,
  RenderMode,
  CMD_SIZE,
  type ReadyMessage,
  type WorkerMessage,
} from "./protocol";

// Initial command buffer capacity (bytes). Grows automatically if needed.
const INITIAL_CAPACITY = 4096;

export class RendererBridge {
  private readonly _worker: Worker;

  /** SharedArrayBuffer written every frame by the worker — high-freq reads. */
  readonly ringBuffer: SharedArrayBuffer;

  /** SharedArrayBuffer written on request by the worker — low-freq reads. */
  readonly snapshotBuffer: SharedArrayBuffer;

  private _buf: ArrayBuffer;
  private _view: DataView;
  private _offset: number = 0;

  private constructor(
    worker: Worker,
    ringBuffer: SharedArrayBuffer,
    snapshotBuffer: SharedArrayBuffer,
  ) {
    this._worker       = worker;
    this.ringBuffer    = ringBuffer;
    this.snapshotBuffer = snapshotBuffer;
    this._buf          = new ArrayBuffer(INITIAL_CAPACITY);
    this._view         = new DataView(this._buf);
  }

  // -------------------------------------------------------------------------
  // Factory — waits for the worker "ready" message before resolving.
  // Returns null if WebGPU is not present (future: return null if SAB
  // construction fails due to missing COOP/COEP headers).
  // -------------------------------------------------------------------------

  static create(): Promise<RendererBridge | null> {
    if (typeof SharedArrayBuffer === "undefined") {
      console.warn(
        "[RendererBridge] SharedArrayBuffer unavailable. " +
        "COOP/COEP headers may not be set. Renderer worker disabled.",
      );
      return Promise.resolve(null);
    }

    return new Promise((resolve) => {
      const worker = new Worker(
        new URL("./renderer.worker.ts", import.meta.url),
        { type: "module" },
      );

      const onMessage = (event: MessageEvent<WorkerMessage>) => {
        const msg = event.data;
        if (msg.type === "ready") {
          worker.removeEventListener("message", onMessage);
          resolve(new RendererBridge(worker, msg.ringBuffer, msg.snapshotBuffer));
        } else if (msg.type === "error") {
          worker.removeEventListener("message", onMessage);
          console.error("[RendererBridge] worker init error:", msg.message);
          worker.terminate();
          resolve(null);
        }
      };

      worker.addEventListener("message", onMessage);

      worker.addEventListener("error", (e) => {
        console.error("[RendererBridge] worker error:", e.message);
        resolve(null);
      }, { once: true });
    });
  }

  // -------------------------------------------------------------------------
  // Command builders — each writes directly into the internal buffer and
  // returns `this` for chaining. Call flush() to transfer.
  // -------------------------------------------------------------------------

  loadChunk(chunkId: number, slotIndex: number): this {
    this._ensure(CMD_SIZE.LoadChunk);
    this._view.setUint8(this._offset, RendererOpcode.LoadChunk);
    this._view.setUint32(this._offset + 1, chunkId,   true);
    this._view.setUint16(this._offset + 5, slotIndex, true);
    this._offset += CMD_SIZE.LoadChunk;
    return this;
  }

  unloadChunk(chunkId: number): this {
    this._ensure(CMD_SIZE.UnloadChunk);
    this._view.setUint8(this._offset, RendererOpcode.UnloadChunk);
    this._view.setUint32(this._offset + 1, chunkId, true);
    this._offset += CMD_SIZE.UnloadChunk;
    return this;
  }

  setCamera(
    pos: readonly [number, number, number],
    dir: readonly [number, number, number],
  ): this {
    this._ensure(CMD_SIZE.SetCamera);
    this._view.setUint8(   this._offset,      RendererOpcode.SetCamera);
    this._view.setFloat32( this._offset +  1, pos[0], true);
    this._view.setFloat32( this._offset +  5, pos[1], true);
    this._view.setFloat32( this._offset +  9, pos[2], true);
    this._view.setFloat32( this._offset + 13, dir[0], true);
    this._view.setFloat32( this._offset + 17, dir[1], true);
    this._view.setFloat32( this._offset + 21, dir[2], true);
    this._offset += CMD_SIZE.SetCamera;
    return this;
  }

  setRenderMode(mode: RenderMode): this {
    this._ensure(CMD_SIZE.SetRenderMode);
    this._view.setUint8(this._offset,     RendererOpcode.SetRenderMode);
    this._view.setUint8(this._offset + 1, mode);
    this._offset += CMD_SIZE.SetRenderMode;
    return this;
  }

  resizeViewport(width: number, height: number): this {
    this._ensure(CMD_SIZE.ResizeViewport);
    this._view.setUint8(  this._offset,     RendererOpcode.ResizeViewport);
    this._view.setUint16( this._offset + 1, width,  true);
    this._view.setUint16( this._offset + 3, height, true);
    this._offset += CMD_SIZE.ResizeViewport;
    return this;
  }

  // -------------------------------------------------------------------------
  // flush — transfer the pending command buffer to the worker (zero-copy).
  // The internal buffer is replaced so the next batch can accumulate.
  // -------------------------------------------------------------------------

  flush(): void {
    if (this._offset === 0) return;
    const transfer = this._buf.slice(0, this._offset);
    this._worker.postMessage({ type: "commands", buffer: transfer }, [transfer]);
    this._offset = 0;
    // Reuse the original allocation for the next batch
    this._buf  = new ArrayBuffer(INITIAL_CAPACITY);
    this._view = new DataView(this._buf);
  }

  // -------------------------------------------------------------------------
  // Lifecycle
  // -------------------------------------------------------------------------

  terminate(): void {
    this._worker.terminate();
  }

  // -------------------------------------------------------------------------
  // Internal: grow buffer if remaining capacity is insufficient
  // -------------------------------------------------------------------------

  private _ensure(needed: number): void {
    const remaining = this._buf.byteLength - this._offset;
    if (remaining >= needed) return;

    const newSize = Math.max(this._buf.byteLength * 2, this._offset + needed);
    const next    = new ArrayBuffer(newSize);
    new Uint8Array(next).set(new Uint8Array(this._buf, 0, this._offset));
    this._buf  = next;
    this._view = new DataView(next);
  }
}
