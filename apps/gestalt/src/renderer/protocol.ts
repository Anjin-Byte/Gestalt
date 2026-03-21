/**
 * Renderer Boundary Protocol
 *
 * Typed binary contract between the main thread (Svelte) and the renderer
 * worker. All communication is packed binary — no JSON, no JS object
 * serialization at the boundary.
 *
 * See: docs/architecture/wasm-boundary-protocol.md
 *
 * ─────────────────────────────────────────────────────────────────────────────
 * COMMAND QUEUE  (main → worker)
 * Transport: Transferable ArrayBuffer posted to the renderer worker.
 * Format: packed sequence of [opcode: u8, ...payload bytes].
 * ─────────────────────────────────────────────────────────────────────────────
 *
 * Each command:
 *
 *  LoadChunk       0x01  chunkId:u32  slotIndex:u16    → 7 bytes
 *  UnloadChunk     0x02  chunkId:u32                   → 5 bytes
 *  SetCamera       0x03  posX:f32 posY:f32 posZ:f32
 *                        dirX:f32 dirY:f32 dirZ:f32    → 25 bytes
 *  SetRenderMode   0x04  mode:u8                       → 2 bytes
 *  ResizeViewport  0x05  width:u16 height:u16          → 5 bytes
 *
 * ─────────────────────────────────────────────────────────────────────────────
 * STATE READBACK  (worker → main)
 * High-freq: SharedArrayBuffer ring buffer, written every frame.
 * Low-freq:  SharedArrayBuffer fixed-layout snapshot, written on request.
 * ─────────────────────────────────────────────────────────────────────────────
 */

// ---------------------------------------------------------------------------
// Opcodes
// ---------------------------------------------------------------------------

export const RendererOpcode = {
  LoadChunk:      0x01,
  UnloadChunk:    0x02,
  SetCamera:      0x03,
  SetRenderMode:  0x04,
  ResizeViewport: 0x05,
} as const;

export type RendererOpcode = (typeof RendererOpcode)[keyof typeof RendererOpcode];

// ---------------------------------------------------------------------------
// Command byte sizes (opcode byte included)
// ---------------------------------------------------------------------------

export const CMD_SIZE = {
  LoadChunk:      1 + 4 + 2,   // opcode + chunkId:u32 + slotIndex:u16
  UnloadChunk:    1 + 4,        // opcode + chunkId:u32
  SetCamera:      1 + 4 * 6,   // opcode + 6 × f32
  SetRenderMode:  1 + 1,        // opcode + mode:u8
  ResizeViewport: 1 + 2 + 2,   // opcode + width:u16 + height:u16
} as const;

// ---------------------------------------------------------------------------
// Render modes
// ---------------------------------------------------------------------------

export const RenderMode = {
  Default:   0x00,
  Wireframe: 0x01,
  Normals:   0x02,
  Depth:     0x03,
} as const;

export type RenderMode = (typeof RenderMode)[keyof typeof RenderMode];

// ---------------------------------------------------------------------------
// High-frequency ring buffer layout (SharedArrayBuffer)
//
// [0..3]   head: u32          — frame index of the most recently written slot
// [4..7]   capacity: u32      — number of slots in the ring
// [8..]    ring entries       — capacity × FRAME_STRIDE bytes
//
// Each FRAME_STRIDE entry:
//   [0..3]   totalMs: f32
//   [4..7]   passCount: u32
//   [8..]    MAX_PASSES × PASS_ENTRY_SIZE bytes
//     passEntry: nameHash:u32  ms:f32   → 8 bytes each
// ---------------------------------------------------------------------------

export const RING_CAPACITY    = 240;  // matches HISTORY_FRAMES in stores/timeline.ts
export const MAX_PASSES       = 8;
export const PASS_ENTRY_SIZE  = 4 + 4;                                    // nameHash:u32 + ms:f32
export const FRAME_STRIDE     = 4 + 4 + MAX_PASSES * PASS_ENTRY_SIZE;    // 72 bytes
export const RING_HEADER_SIZE = 4 + 4;                                    //  8 bytes
export const RING_BUFFER_SIZE = RING_HEADER_SIZE + RING_CAPACITY * FRAME_STRIDE; // 17288 bytes

// Byte offsets within the ring header
export const RING_OFFSET_HEAD     = 0;
export const RING_OFFSET_CAPACITY = 4;

// Byte offsets within each FRAME_STRIDE entry
export const FRAME_OFFSET_TOTAL_MS   = 0;
export const FRAME_OFFSET_PASS_COUNT = 4;
export const FRAME_OFFSET_PASSES     = 8;

// ---------------------------------------------------------------------------
// Low-frequency snapshot layout (SharedArrayBuffer)
//
// Fixed-size buffer written by WASM on request; read by JS with DataView.
// No allocation, no GC pressure.
// Layout is append-only — new fields go at the end with a version bump.
// ---------------------------------------------------------------------------

export const SNAPSHOT_VERSION        = 1;
export const MAX_CHUNKS_IN_SNAPSHOT  = 1024;
export const CHUNK_ENTRY_SIZE        = 4 + 2;  // chunkId:u32 + slotIndex:u16
export const SNAPSHOT_HEADER_SIZE    = 4 + 4;  // version:u32 + chunkCount:u32
export const SNAPSHOT_BUFFER_SIZE    =
  SNAPSHOT_HEADER_SIZE + MAX_CHUNKS_IN_SNAPSHOT * CHUNK_ENTRY_SIZE; // 6152 bytes

export const SNAPSHOT_OFFSET_VERSION     = 0;
export const SNAPSHOT_OFFSET_CHUNK_COUNT = 4;
export const SNAPSHOT_OFFSET_CHUNKS      = 8;

// ---------------------------------------------------------------------------
// Worker message shapes (typed wrappers over the raw buffers)
// ---------------------------------------------------------------------------

/** Main thread → worker: a batch of packed commands. */
export interface CommandMessage {
  type: "commands";
  buffer: ArrayBuffer;  // transferred (Transferable), not copied
}

/** Worker → main thread: frame complete, ring buffer updated. */
export interface FrameMessage {
  type: "frame";
  frameIndex: number;
}

/** Worker → main thread: snapshot written on request. */
export interface SnapshotMessage {
  type: "snapshot";
}

/** Worker → main thread: initialization complete + SAB handles. */
export interface ReadyMessage {
  type: "ready";
  ringBuffer: SharedArrayBuffer;
  snapshotBuffer: SharedArrayBuffer;
}

/** Worker → main thread: unrecoverable error. */
export interface ErrorMessage {
  type: "error";
  message: string;
}

export type WorkerMessage =
  | ReadyMessage
  | FrameMessage
  | SnapshotMessage
  | ErrorMessage;

export type MainMessage = CommandMessage;
