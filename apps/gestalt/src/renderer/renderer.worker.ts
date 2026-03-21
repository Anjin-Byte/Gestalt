/**
 * Renderer Worker
 *
 * This worker will own the WebGPU device and the frame loop.
 * It receives packed command buffers from the main thread and writes frame
 * state back via SharedArrayBuffer ring buffer (high-freq) and snapshot
 * (low-freq).
 *
 * Current status: STUB — receives and decodes commands, allocates the SABs,
 * posts a "ready" message. No WASM or WebGPU wired yet.
 *
 * See: docs/architecture/wasm-boundary-protocol.md
 */

import {
  type CommandMessage,
  type WorkerMessage,
  RendererOpcode,
  CMD_SIZE,
  RING_BUFFER_SIZE,
  RING_CAPACITY,
  RING_HEADER_SIZE,
  RING_OFFSET_CAPACITY,
  SNAPSHOT_BUFFER_SIZE,
} from "./protocol";

// ---------------------------------------------------------------------------
// Shared buffers — allocated once, never resized
// ---------------------------------------------------------------------------

const ringBuffer      = new SharedArrayBuffer(RING_BUFFER_SIZE);
const snapshotBuffer  = new SharedArrayBuffer(SNAPSHOT_BUFFER_SIZE);

// Initialise ring header: capacity field
const ringView = new DataView(ringBuffer);
ringView.setUint32(RING_HEADER_SIZE - 4, RING_CAPACITY, true); // capacity at offset 4

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function post(msg: WorkerMessage): void {
  self.postMessage(msg);
}

// ---------------------------------------------------------------------------
// Command decoder
// Reads a packed command buffer and dispatches each command.
// The switch is exhaustive over the protocol opcodes; unknown opcodes log a
// warning and skip to the next (recovery by advancing past unknown payload
// is not possible without a length prefix — unknown opcodes halt decoding).
// ---------------------------------------------------------------------------

function decodeCommands(buffer: ArrayBuffer): void {
  const view   = new DataView(buffer);
  let   offset = 0;

  while (offset < buffer.byteLength) {
    const opcode = view.getUint8(offset);

    switch (opcode) {
      case RendererOpcode.LoadChunk: {
        const chunkId   = view.getUint32(offset + 1, true);
        const slotIndex = view.getUint16(offset + 5, true);
        onLoadChunk(chunkId, slotIndex);
        offset += CMD_SIZE.LoadChunk;
        break;
      }
      case RendererOpcode.UnloadChunk: {
        const chunkId = view.getUint32(offset + 1, true);
        onUnloadChunk(chunkId);
        offset += CMD_SIZE.UnloadChunk;
        break;
      }
      case RendererOpcode.SetCamera: {
        const posX = view.getFloat32(offset +  1, true);
        const posY = view.getFloat32(offset +  5, true);
        const posZ = view.getFloat32(offset +  9, true);
        const dirX = view.getFloat32(offset + 13, true);
        const dirY = view.getFloat32(offset + 17, true);
        const dirZ = view.getFloat32(offset + 21, true);
        onSetCamera(posX, posY, posZ, dirX, dirY, dirZ);
        offset += CMD_SIZE.SetCamera;
        break;
      }
      case RendererOpcode.SetRenderMode: {
        const mode = view.getUint8(offset + 1);
        onSetRenderMode(mode);
        offset += CMD_SIZE.SetRenderMode;
        break;
      }
      case RendererOpcode.ResizeViewport: {
        const width  = view.getUint16(offset + 1, true);
        const height = view.getUint16(offset + 3, true);
        onResizeViewport(width, height);
        offset += CMD_SIZE.ResizeViewport;
        break;
      }
      default:
        // Unknown opcode: cannot recover (no length prefix) — halt decoding.
        console.warn(`[renderer.worker] unknown opcode 0x${opcode.toString(16)} at offset ${offset}`);
        return;
    }
  }
}

// ---------------------------------------------------------------------------
// Command handlers (stubs — filled in when WASM is wired)
// ---------------------------------------------------------------------------

function onLoadChunk(_chunkId: number, _slotIndex: number): void {
  // TODO: forward to WASM chunk manager
}

function onUnloadChunk(_chunkId: number): void {
  // TODO: forward to WASM chunk manager
}

function onSetCamera(
  _posX: number, _posY: number, _posZ: number,
  _dirX: number, _dirY: number, _dirZ: number,
): void {
  // TODO: update renderer camera uniform
}

function onSetRenderMode(_mode: number): void {
  // TODO: set renderer pipeline variant
}

function onResizeViewport(_width: number, _height: number): void {
  // TODO: resize WebGPU swap-chain and depth buffer
}

// ---------------------------------------------------------------------------
// Message listener
// ---------------------------------------------------------------------------

self.addEventListener("message", (event: MessageEvent<CommandMessage>) => {
  const msg = event.data;
  if (msg.type === "commands") {
    decodeCommands(msg.buffer);
  }
});

// ---------------------------------------------------------------------------
// Signal ready — send SAB handles to main thread
// ---------------------------------------------------------------------------

post({
  type: "ready",
  ringBuffer,
  snapshotBuffer,
});
