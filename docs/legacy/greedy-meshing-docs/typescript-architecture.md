# TypeScript Architecture

> **Part of the Voxel Mesh Architecture**
>
> TypeScript patterns, type safety, and debugging infrastructure for the voxel mesh system.
>
> Related documents:
> - [Development Guidelines](development-guidelines.md) - General coding standards
> - [Voxel Mesh Architecture](voxel-mesh-architecture.md) - System overview
> - [Chunk Management System](chunk-management-system.md) - State machine design

---

## Core Principles

1. **Type Safety First**: Leverage TypeScript's type system to catch errors at compile time
2. **Explicit State**: Use discriminated unions for all state machines
3. **Debuggable by Default**: Built-in logging, tracing, and inspection hooks
4. **Resource Safety**: Explicit lifecycle management for Three.js objects

---

## Type Patterns

### Branded Types

Prevent accidentally mixing up numeric IDs:

```typescript
// Brand symbols (never instantiated, just for type checking)
declare const ChunkIdBrand: unique symbol;
declare const MaterialIdBrand: unique symbol;
declare const VersionBrand: unique symbol;

// Branded types
export type ChunkId = number & { readonly [ChunkIdBrand]: never };
export type MaterialId = number & { readonly [MaterialIdBrand]: never };
export type Version = number & { readonly [VersionBrand]: never };

// Factory functions (the only way to create these types)
export function chunkId(x: number, y: number, z: number): ChunkId {
    // Pack into single number for Map key efficiency
    // Supports coordinates -524288 to 524287 (20 bits each)
    const packed = ((x + 524288) | ((y + 524288) << 20) | ((z + 524288) << 40));
    return packed as ChunkId;
}

export function materialId(id: number): MaterialId {
    if (id < 0 || id > 65535) {
        throw new RangeError(`Material ID must be 0-65535, got ${id}`);
    }
    return id as MaterialId;
}

export function version(v: number): Version {
    return v as Version;
}

// Extract coordinates from ChunkId
export function chunkCoord(id: ChunkId): ChunkCoord {
    const n = id as number;
    return {
        x: (n & 0xFFFFF) - 524288,
        y: ((n >> 20) & 0xFFFFF) - 524288,
        z: ((n >> 40) & 0xFFFFF) - 524288,
    };
}
```

**Usage:**
```typescript
// Compile-time errors:
const mat: MaterialId = 5;           // Error: number not assignable to MaterialId
const chunk: ChunkId = materialId(5); // Error: MaterialId not assignable to ChunkId

// Correct usage:
const mat: MaterialId = materialId(5);      // OK
const chunk: ChunkId = chunkId(1, 2, 3);   // OK
```

### Discriminated Unions for State

```typescript
// Chunk state machine
export type ChunkState =
    | { kind: 'empty' }
    | { kind: 'loading'; startTime: number }
    | { kind: 'clean'; mesh: ChunkMesh; version: Version }
    | { kind: 'dirty'; mesh: ChunkMesh; version: Version; reason: DirtyReason }
    | { kind: 'meshing'; mesh: ChunkMesh; pendingVersion: Version }
    | { kind: 'ready_to_swap'; oldMesh: ChunkMesh; newMesh: ChunkMesh; version: Version };

// Dirty reasons (also discriminated)
export type DirtyReason =
    | { kind: 'voxel_edit'; positions: VoxelPos[] }
    | { kind: 'neighbor_boundary'; neighborId: ChunkId; face: FaceDir }
    | { kind: 'material_change'; materialId: MaterialId }
    | { kind: 'full_rebuild' };

// Type-safe state access
function handleChunkState(chunk: Chunk): void {
    switch (chunk.state.kind) {
        case 'empty':
            // TypeScript knows: no other properties
            break;

        case 'clean':
            // TypeScript knows: chunk.state.mesh and chunk.state.version exist
            renderMesh(chunk.state.mesh);
            break;

        case 'dirty':
            // TypeScript knows: chunk.state.reason exists
            if (chunk.state.reason.kind === 'voxel_edit') {
                console.log(`Edited ${chunk.state.reason.positions.length} voxels`);
            }
            break;

        case 'meshing':
            // Show loading indicator
            break;

        case 'ready_to_swap':
            // TypeScript knows: oldMesh and newMesh both exist
            swapMesh(chunk.state.oldMesh, chunk.state.newMesh);
            break;
    }
}
```

### State Transitions

```typescript
// Valid state transitions (compile-time enforced)
type StateTransition = {
    from: ChunkState['kind'];
    to: ChunkState['kind'];
};

const VALID_TRANSITIONS: StateTransition[] = [
    { from: 'empty', to: 'loading' },
    { from: 'loading', to: 'clean' },
    { from: 'loading', to: 'empty' },  // Load failed
    { from: 'clean', to: 'dirty' },
    { from: 'dirty', to: 'meshing' },
    { from: 'dirty', to: 'dirty' },     // Additional edits
    { from: 'meshing', to: 'ready_to_swap' },
    { from: 'meshing', to: 'dirty' },   // Edit during mesh
    { from: 'ready_to_swap', to: 'clean' },
    { from: 'ready_to_swap', to: 'dirty' }, // Edit before swap
];

function isValidTransition(from: ChunkState['kind'], to: ChunkState['kind']): boolean {
    return VALID_TRANSITIONS.some(t => t.from === from && t.to === to);
}

function transitionTo(chunk: Chunk, newState: ChunkState): void {
    if (!isValidTransition(chunk.state.kind, newState.kind)) {
        throw new ChunkStateError(
            `Invalid state transition: ${chunk.state.kind} -> ${newState.kind}`,
            chunk.id,
            'INVALID_TRANSITION'
        );
    }

    const oldState = chunk.state;
    chunk.state = newState;

    // Emit event for debugging/logging
    events.emit({
        kind: 'state_changed',
        chunkId: chunk.id,
        from: oldState.kind,
        to: newState.kind,
    });
}
```

### Result Types

```typescript
// Explicit success/failure without exceptions for expected errors
export type Result<T, E = Error> =
    | { ok: true; value: T }
    | { ok: false; error: E };

// Helper functions
export function ok<T>(value: T): Result<T, never> {
    return { ok: true, value };
}

export function err<E>(error: E): Result<never, E> {
    return { ok: false, error };
}

export function isOk<T, E>(result: Result<T, E>): result is { ok: true; value: T } {
    return result.ok;
}

export function unwrap<T, E>(result: Result<T, E>): T {
    if (result.ok) {
        return result.value;
    }
    throw result.error;
}

// Usage
function loadChunk(id: ChunkId): Result<Chunk, ChunkError> {
    const data = storage.get(id);
    if (!data) {
        return err(new ChunkError('Chunk not found', id, 'NOT_FOUND'));
    }

    try {
        const chunk = deserialize(data);
        return ok(chunk);
    } catch (e) {
        return err(new ChunkError('Deserialization failed', id, 'DESERIALIZE_FAILED'));
    }
}

// Caller decides how to handle
const result = loadChunk(chunkId(1, 2, 3));
if (result.ok) {
    useChunk(result.value);
} else {
    logger.warn('Failed to load chunk', { error: result.error.message });
}
```

### Custom Error Classes

```typescript
export class ChunkError extends Error {
    constructor(
        message: string,
        public readonly chunkId: ChunkId,
        public readonly code: ChunkErrorCode
    ) {
        super(message);
        this.name = 'ChunkError';
    }
}

export type ChunkErrorCode =
    | 'NOT_FOUND'
    | 'INVALID_TRANSITION'
    | 'MESH_FAILED'
    | 'ALLOCATION_FAILED'
    | 'VERSION_MISMATCH'
    | 'DESERIALIZE_FAILED';

export class MeshError extends Error {
    constructor(
        message: string,
        public readonly code: MeshErrorCode,
        public readonly details?: Record<string, unknown>
    ) {
        super(message);
        this.name = 'MeshError';
    }
}

export type MeshErrorCode =
    | 'WASM_UNAVAILABLE'
    | 'BUFFER_ALLOCATION_FAILED'
    | 'INVALID_INPUT'
    | 'TIMEOUT';
```

---

## Debug Infrastructure

### Structured Logger

```typescript
export type LogLevel = 'debug' | 'info' | 'warn' | 'error';

export interface LogEntry {
    timestamp: number;
    level: LogLevel;
    logger: string;
    message: string;
    context?: Record<string, unknown>;
}

export interface Logger {
    debug(message: string, context?: Record<string, unknown>): void;
    info(message: string, context?: Record<string, unknown>): void;
    warn(message: string, context?: Record<string, unknown>): void;
    error(message: string, context?: Record<string, unknown>): void;

    // Create child logger with prefix
    child(name: string): Logger;

    // Performance timing
    time(label: string): () => number;
}

class StructuredLogger implements Logger {
    private static globalLevel: LogLevel = 'info';
    private static buffer: LogEntry[] = [];
    private static maxBuffer = 1000;

    constructor(
        private readonly name: string,
        private readonly parent?: StructuredLogger
    ) {}

    static setLevel(level: LogLevel): void {
        StructuredLogger.globalLevel = level;
    }

    static getBuffer(): readonly LogEntry[] {
        return StructuredLogger.buffer;
    }

    static clearBuffer(): void {
        StructuredLogger.buffer = [];
    }

    debug(message: string, context?: Record<string, unknown>): void {
        this.log('debug', message, context);
    }

    info(message: string, context?: Record<string, unknown>): void {
        this.log('info', message, context);
    }

    warn(message: string, context?: Record<string, unknown>): void {
        this.log('warn', message, context);
    }

    error(message: string, context?: Record<string, unknown>): void {
        this.log('error', message, context);
    }

    child(name: string): Logger {
        return new StructuredLogger(`${this.fullName()}.${name}`, this);
    }

    time(label: string): () => number {
        const start = performance.now();
        return () => {
            const duration = performance.now() - start;
            this.debug(`${label}`, { durationMs: duration.toFixed(2) });
            return duration;
        };
    }

    private fullName(): string {
        return this.parent ? `${this.parent.fullName()}.${this.name}` : this.name;
    }

    private log(level: LogLevel, message: string, context?: Record<string, unknown>): void {
        if (!this.shouldLog(level)) return;

        const entry: LogEntry = {
            timestamp: Date.now(),
            level,
            logger: this.fullName(),
            message,
            context,
        };

        // Buffer for inspection
        StructuredLogger.buffer.push(entry);
        if (StructuredLogger.buffer.length > StructuredLogger.maxBuffer) {
            StructuredLogger.buffer.shift();
        }

        // Console output
        const prefix = `[${entry.logger}]`;
        const contextStr = context ? ` ${JSON.stringify(context)}` : '';
        console[level](`${prefix} ${message}${contextStr}`);
    }

    private shouldLog(level: LogLevel): boolean {
        const levels: LogLevel[] = ['debug', 'info', 'warn', 'error'];
        return levels.indexOf(level) >= levels.indexOf(StructuredLogger.globalLevel);
    }
}

// Create root logger
export const logger = new StructuredLogger('voxel');
```

### Performance Tracing

```typescript
export interface FrameStats {
    frameId: number;
    totalMs: number;
    sections: Record<string, number>;
}

export class PerformanceTracer {
    private frameId = 0;
    private currentFrame: Map<string, number> | null = null;
    private frameStart = 0;
    private history: FrameStats[] = [];
    private maxHistory = 60;

    beginFrame(): void {
        this.frameId++;
        this.frameStart = performance.now();
        this.currentFrame = new Map();
    }

    section(name: string): () => void {
        if (!this.currentFrame) {
            return () => {};
        }

        const start = performance.now();
        return () => {
            const duration = performance.now() - start;
            const existing = this.currentFrame!.get(name) ?? 0;
            this.currentFrame!.set(name, existing + duration);
        };
    }

    endFrame(): FrameStats {
        const totalMs = performance.now() - this.frameStart;
        const sections: Record<string, number> = {};

        if (this.currentFrame) {
            for (const [name, duration] of this.currentFrame) {
                sections[name] = duration;
            }
        }

        const stats: FrameStats = {
            frameId: this.frameId,
            totalMs,
            sections,
        };

        this.history.push(stats);
        if (this.history.length > this.maxHistory) {
            this.history.shift();
        }

        this.currentFrame = null;
        return stats;
    }

    getHistory(): readonly FrameStats[] {
        return this.history;
    }

    getAverages(): Record<string, number> {
        if (this.history.length === 0) return {};

        const totals: Record<string, number> = {};
        const counts: Record<string, number> = {};

        for (const frame of this.history) {
            for (const [name, duration] of Object.entries(frame.sections)) {
                totals[name] = (totals[name] ?? 0) + duration;
                counts[name] = (counts[name] ?? 0) + 1;
            }
        }

        const averages: Record<string, number> = {};
        for (const name of Object.keys(totals)) {
            averages[name] = totals[name] / counts[name];
        }
        return averages;
    }
}

export const perf = new PerformanceTracer();
```

### Inspector API

```typescript
export interface VoxelInspector {
    // Chunk inspection
    getChunk(id: ChunkId): Chunk | undefined;
    getChunkState(id: ChunkId): ChunkState | undefined;
    getAllChunkIds(): ChunkId[];
    getChunkCount(): number;

    // Mesh statistics
    getMeshStats(id: ChunkId): MeshStats | undefined;
    getTotalTriangles(): number;
    getTotalVertices(): number;

    // Material inspection
    getMaterial(id: MaterialId): MaterialDef | undefined;
    getAllMaterialIds(): MaterialId[];

    // Queue inspection
    getDirtyQueue(): ChunkId[];
    getMeshingQueue(): ChunkId[];

    // Performance
    getFrameStats(): FrameStats | undefined;
    getAverageFrameStats(): Record<string, number>;
    getLogBuffer(): readonly LogEntry[];

    // Memory
    getMemoryStats(): MemoryStats;

    // Debug visualization
    debugOptions: {
        showChunkBoundaries: boolean;
        showMeshWireframes: boolean;
        showChunkStates: boolean;
        highlightDirtyChunks: boolean;
        logStateTransitions: boolean;
    };
}

export interface MeshStats {
    triangleCount: number;
    vertexCount: number;
    meshTimeMs: number;
    lastMeshVersion: Version;
}

export interface MemoryStats {
    chunkCount: number;
    totalGeometryBytes: number;
    pooledGeometryCount: number;
    wasmHeapBytes: number;
}

// Global inspector (development only)
declare global {
    interface Window {
        __VOXEL_INSPECTOR__?: VoxelInspector;
    }
}

export function installInspector(inspector: VoxelInspector): void {
    if (typeof window !== 'undefined') {
        window.__VOXEL_INSPECTOR__ = inspector;
    }
}
```

---

## Event System

```typescript
// All voxel system events
export type VoxelEvent =
    | { kind: 'chunk_created'; chunkId: ChunkId }
    | { kind: 'chunk_disposed'; chunkId: ChunkId }
    | { kind: 'state_changed'; chunkId: ChunkId; from: ChunkState['kind']; to: ChunkState['kind'] }
    | { kind: 'chunk_meshed'; chunkId: ChunkId; stats: MeshStats }
    | { kind: 'chunk_swapped'; chunkId: ChunkId }
    | { kind: 'voxel_edited'; chunkId: ChunkId; count: number }
    | { kind: 'material_changed'; materialId: MaterialId }
    | { kind: 'error'; error: Error; context?: string };

// Type-safe event emitter
export class VoxelEventEmitter {
    private handlers = new Map<VoxelEvent['kind'], Set<(event: VoxelEvent) => void>>();

    on<K extends VoxelEvent['kind']>(
        kind: K,
        handler: (event: Extract<VoxelEvent, { kind: K }>) => void
    ): () => void {
        if (!this.handlers.has(kind)) {
            this.handlers.set(kind, new Set());
        }

        const typedHandler = handler as (event: VoxelEvent) => void;
        this.handlers.get(kind)!.add(typedHandler);

        // Return unsubscribe function
        return () => {
            this.handlers.get(kind)?.delete(typedHandler);
        };
    }

    emit(event: VoxelEvent): void {
        const handlers = this.handlers.get(event.kind);
        if (handlers) {
            for (const handler of handlers) {
                try {
                    handler(event);
                } catch (e) {
                    console.error('Event handler error:', e);
                }
            }
        }
    }
}

export const events = new VoxelEventEmitter();
```

---

## Resource Lifecycle

### Disposable Pattern

```typescript
export interface Disposable {
    dispose(): void;
}

// Check if object is disposable
export function isDisposable(obj: unknown): obj is Disposable {
    return typeof obj === 'object' && obj !== null && 'dispose' in obj;
}

// Track and dispose Three.js resources
export class ResourceTracker implements Disposable {
    private resources = new Set<Disposable>();
    private disposed = false;

    track<T extends Disposable>(resource: T): T {
        if (this.disposed) {
            throw new Error('Cannot track resource on disposed tracker');
        }
        this.resources.add(resource);
        return resource;
    }

    untrack(resource: Disposable): void {
        this.resources.delete(resource);
    }

    disposeResource(resource: Disposable): void {
        resource.dispose();
        this.resources.delete(resource);
    }

    dispose(): void {
        if (this.disposed) return;
        this.disposed = true;

        for (const resource of this.resources) {
            try {
                resource.dispose();
            } catch (e) {
                console.error('Error disposing resource:', e);
            }
        }
        this.resources.clear();
    }

    get pendingCount(): number {
        return this.resources.size;
    }

    get isDisposed(): boolean {
        return this.disposed;
    }
}
```

### Three.js Resource Helpers

```typescript
// Dispose BufferGeometry and its attributes
export function disposeGeometry(geometry: THREE.BufferGeometry): void {
    geometry.dispose();

    // Dispose any associated buffers
    for (const attr of Object.values(geometry.attributes)) {
        if (attr instanceof THREE.BufferAttribute) {
            // BufferAttribute doesn't have dispose, but we can null the array
            (attr.array as unknown) = null;
        }
    }

    if (geometry.index) {
        (geometry.index.array as unknown) = null;
    }
}

// Dispose material and its textures
export function disposeMaterial(material: THREE.Material): void {
    material.dispose();

    // Dispose textures if any
    if ('map' in material && material.map) {
        (material.map as THREE.Texture).dispose();
    }
    if ('normalMap' in material && material.normalMap) {
        (material.normalMap as THREE.Texture).dispose();
    }
    // ... other texture properties
}

// Dispose mesh completely
export function disposeMesh(mesh: THREE.Mesh): void {
    if (mesh.geometry) {
        disposeGeometry(mesh.geometry);
    }

    if (mesh.material) {
        if (Array.isArray(mesh.material)) {
            mesh.material.forEach(disposeMaterial);
        } else {
            disposeMaterial(mesh.material);
        }
    }

    // Remove from parent
    mesh.removeFromParent();
}
```

---

## Module Organization

```
apps/web/src/voxel/
├── index.ts                    # Public API
├── types.ts                    # All type definitions
├── constants.ts                # CS_P, FACE_*, etc.
├── errors.ts                   # Error classes
├── events.ts                   # Event emitter and types
│
├── chunk/
│   ├── index.ts               # Chunk subsystem exports
│   ├── Chunk.ts               # Chunk class
│   ├── ChunkId.ts             # Branded type and utilities
│   ├── ChunkManager.ts        # Main orchestrator
│   ├── ChunkState.ts          # State machine
│   ├── DirtyTracker.ts        # Edit tracking
│   └── RebuildScheduler.ts    # Priority queue
│
├── mesh/
│   ├── index.ts
│   ├── ChunkMesh.ts           # Mesh wrapper
│   ├── ChunkMeshPool.ts       # Geometry pooling
│   ├── DoubleBuffer.ts        # Swap mechanism
│   ├── GeometryBuilder.ts     # WASM -> BufferGeometry
│   └── MaterialManager.ts     # Atlas management
│
├── edit/
│   ├── index.ts
│   ├── VoxelEditor.ts         # Edit API
│   ├── EditHistory.ts         # Undo/redo
│   └── Brush.ts               # Brush tools
│
├── wasm/
│   ├── index.ts
│   ├── WasmBridge.ts          # Module loading
│   └── WorkerPool.ts          # Threading
│
├── debug/
│   ├── index.ts
│   ├── Inspector.ts           # VoxelInspector impl
│   ├── Logger.ts              # StructuredLogger
│   ├── PerformanceTracer.ts   # Timing
│   ├── DebugViews.ts          # Visualization helpers
│   └── DebugPanel.ts          # UI component
│
└── __tests__/
    ├── ChunkState.test.ts
    ├── DirtyTracker.test.ts
    └── ResourceTracker.test.ts
```

### Public API (index.ts)

```typescript
// Types
export type {
    ChunkId,
    ChunkCoord,
    ChunkState,
    MaterialId,
    MaterialDef,
    VoxelPos,
    VoxelEvent,
    MeshStats,
    FrameStats,
} from './types';

// Core classes
export { ChunkManager } from './chunk/ChunkManager';
export { VoxelEditor } from './edit/VoxelEditor';
export { MaterialManager } from './mesh/MaterialManager';

// Utilities
export { chunkId, chunkCoord, materialId } from './chunk/ChunkId';
export { events } from './events';
export { logger } from './debug/Logger';
export { perf } from './debug/PerformanceTracer';

// Debug (tree-shakeable in production)
export { installInspector } from './debug/Inspector';
```

---

## Testing Patterns

### Unit Test Structure

```typescript
// ChunkState.test.ts
import { describe, it, expect } from 'vitest';
import { transitionTo, isValidTransition } from './ChunkState';
import { chunkId, version } from './ChunkId';

describe('ChunkState', () => {
    describe('isValidTransition', () => {
        it('allows empty -> loading', () => {
            expect(isValidTransition('empty', 'loading')).toBe(true);
        });

        it('rejects empty -> clean', () => {
            expect(isValidTransition('empty', 'clean')).toBe(false);
        });
    });

    describe('transitionTo', () => {
        it('throws on invalid transition', () => {
            const chunk = createChunk({ kind: 'empty' });

            expect(() => {
                transitionTo(chunk, { kind: 'meshing', mesh: mockMesh(), pendingVersion: version(1) });
            }).toThrow('Invalid state transition: empty -> meshing');
        });

        it('updates state on valid transition', () => {
            const chunk = createChunk({ kind: 'empty' });

            transitionTo(chunk, { kind: 'loading', startTime: Date.now() });

            expect(chunk.state.kind).toBe('loading');
        });
    });
});
```

### Integration Test Example

```typescript
// ChunkManager.integration.test.ts
describe('ChunkManager integration', () => {
    it('marks neighbors dirty on boundary edit', async () => {
        const manager = new ChunkManager();

        // Load a 2x1x1 strip of chunks
        await manager.loadChunk(chunkId(0, 0, 0));
        await manager.loadChunk(chunkId(1, 0, 0));

        // Edit voxel at boundary (x=63)
        const editor = new VoxelEditor(manager);
        editor.setVoxel({ x: 63, y: 32, z: 32 }, materialId(1));

        // Both chunks should be dirty
        expect(manager.getChunkState(chunkId(0, 0, 0))?.kind).toBe('dirty');
        expect(manager.getChunkState(chunkId(1, 0, 0))?.kind).toBe('dirty');
    });
});
```

---

## Summary

| Pattern | Purpose |
|---------|---------|
| Branded types | Prevent ID type confusion |
| Discriminated unions | Type-safe state machines |
| Result types | Explicit error handling |
| Structured logging | Debuggable output |
| Performance tracing | Frame-level profiling |
| Inspector API | Runtime debugging |
| Event system | Decoupled communication |
| Resource tracker | Prevent memory leaks |
