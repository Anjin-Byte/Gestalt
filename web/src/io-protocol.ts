// The IO-worker job protocol, shared by the facade (io.ts) and the worker
// (worker.ts). Internal to this app: both ends build from the same source, so
// changing a shape here rebuilds both sides together
// (docs/design/web-frontend-api.md §5, stage 7).
import { MeshFormat } from "voxel-web";

/** Every job's observable phase keys — mirrored from the kernel's `Phase::key`
 * (the kernel names what is happening; the shell owns the ordering and the
 * human wording). One union across all jobs so the shell renders one segmented
 * bar; each operation emits its own ordered subset (see the `*_PHASES` arrays).
 */
export type ProgressPhase =
  | "parse"
  | "voxelize"
  | "compact"
  | "cutout"
  | "assemble"
  | "colorBake"
  | "generate"
  | "gather"
  | "write"
  // Serializing the built scene into the transfer blob — metered by bytes.
  | "pack"
  // Render-worker scene upload. Emitted by the SHELL, not the kernel: that
  // step crosses a worker whose protocol carries no progress counts, so the
  // shell reports it indeterminately around its own `installScene` await.
  | "install";

/** One progress event: `total === 0` marks an indeterminate phase. */
export interface JobProgress {
  readonly phase: ProgressPhase;
  readonly done: number;
  readonly total: number;
}

/** The ordered phases each job reports — the segmented bar's segment list. A
 * job's stream visits a prefix/subset of its array (a phase may be skipped, and
 * the phases before the active one read as done). */
export const MESH_PHASES: readonly ProgressPhase[] = [
  "parse",
  "voxelize",
  "compact",
  "cutout",
  "assemble",
  "colorBake",
  "pack",
  "install",
];
/** CPU-scan or GPU-noise fixture build. */
export const FIXTURE_PHASES: readonly ProgressPhase[] = [
  "generate",
  "assemble",
  "pack",
  "install",
];
/** `.vox`/`.cvox` decode. */
export const DECODE_PHASES: readonly ProgressPhase[] = [
  "parse",
  "assemble",
  "pack",
  "install",
];
/** `.vox`/`.cvox` export. */
export const ENCODE_PHASES: readonly ProgressPhase[] = ["gather", "write"];

/** What an imported filename routes to: the GPU voxelizer for mesh formats,
 * the direct voxel-native loaders for `.vox`/`.cvox`. */
export type ImportKind =
  | { readonly kind: "mesh"; readonly format: MeshFormat }
  | { readonly kind: "vox" }
  | { readonly kind: "cvox" };

export function importKindFor(name: string): ImportKind | undefined {
  const ext = name.slice(name.lastIndexOf(".") + 1).toLowerCase();
  switch (ext) {
    case "glb":
    case "gltf":
      return { kind: "mesh", format: MeshFormat.Glb };
    case "obj":
      return { kind: "mesh", format: MeshFormat.Obj };
    case "stl":
      return { kind: "mesh", format: MeshFormat.Stl };
    case "vox":
      return { kind: "vox" };
    case "cvox":
      return { kind: "cvox" };
    default:
      return undefined;
  }
}

/** Options forwarded to the kernel's mesh voxelizer. */
export interface VoxelizeOptions {
  readonly res: number;
  readonly truecolor: boolean;
  readonly rotX: number;
  /** Bake colours on the GPU (CPU-oracle fallback); `false` = the A/B path. */
  readonly gpuBake: boolean;
}

export type Job =
  | {
      readonly kind: "voxelizeMesh";
      readonly bytes: Uint8Array;
      readonly format: MeshFormat;
      readonly opts: VoxelizeOptions;
    }
  | { readonly kind: "buildFixture"; readonly fixture: string; readonly res: number }
  | { readonly kind: "decodeVox" | "decodeCvox"; readonly bytes: Uint8Array }
  | { readonly kind: "encodeVox" | "encodeCvox"; readonly scene: Uint8Array };

export interface JobRequest {
  readonly id: number;
  readonly job: Job;
}

export type JobReply =
  | {
      readonly id: number;
      readonly ok: true;
      readonly bytes: Uint8Array;
      /** The worker's wasm heap after the job — the memory-audit gauge
       * (wasm heaps only grow; the client recycles the worker on this). */
      readonly heapBytes: number;
    }
  | {
      readonly id: number;
      readonly ok: false;
      readonly error: string;
      readonly heapBytes: number;
    }
  | { readonly id: number; readonly progress: JobProgress };
