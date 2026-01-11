export type Vec3Tuple = [number, number, number];

export type MeshDescriptor = {
  positions: Float32Array;
  indices?: Uint32Array;
  normals?: Float32Array;
  colors?: Float32Array;
};

export type VoxelsDescriptor = {
  positions: Float32Array<ArrayBufferLike>;
  voxelSize: number;
  color?: Vec3Tuple;
  renderMode?: "cubes" | "points";
  chunkSize?: number;
  pointSize?: number;
};

export type LinesDescriptor = {
  positions: Float32Array;
  color?: Vec3Tuple;
};

export type PointsDescriptor = {
  positions: Float32Array;
  color?: Vec3Tuple;
  size?: number;
};

export type TextureDescriptor = {
  width: number;
  height: number;
  data: Uint8Array;
};

export type ModuleOutput =
  | { kind: "mesh"; mesh: MeshDescriptor; label?: string }
  | { kind: "voxels"; voxels: VoxelsDescriptor; label?: string }
  | { kind: "texture2d"; texture: TextureDescriptor; label?: string }
  | { kind: "lines"; lines: LinesDescriptor; label?: string }
  | { kind: "points"; points: PointsDescriptor; label?: string };

export type RunRequest = {
  params: Record<string, unknown>;
  frameId: number;
};

export type Logger = {
  info: (message: string) => void;
  warn: (message: string) => void;
  error: (message: string) => void;
};

export type ModuleContext = {
  requestGpuDevice: () => Promise<GPUDevice | null>;
  logger: Logger;
  baseUrl: string;
  emitOutputs?: (outputs: ModuleOutput[]) => void;
};

export type UiApi = {
  addSlider: (options: {
    id: string;
    label: string;
    min: number;
    max: number;
    step: number;
    initial: number;
  }) => void;
  addNumber: (options: {
    id: string;
    label: string;
    min: number;
    max: number;
    step: number;
    initial: number;
  }) => void;
  addCheckbox: (options: { id: string; label: string; initial: boolean }) => void;
  addSelect: (options: {
    id: string;
    label: string;
    options: string[];
    initial: string;
  }) => void;
  addText: (options: { id: string; label: string; initial: string }) => void;
  setText: (id: string, value: string) => void;
  addFile: (options: {
    id: string;
    label: string;
    accept: string;
    onFile: (file: File | null) => void | Promise<void>;
  }) => void;
  addButton: (options: { label: string; onClick: () => void }) => void;
  getValues: () => Record<string, unknown>;
  clear: () => void;
};

export type UiControl =
  | {
      kind: "slider";
      id: string;
      label: string;
      min: number;
      max: number;
      step: number;
      initial: number;
    }
  | {
      kind: "number";
      id: string;
      label: string;
      min: number;
      max: number;
      step: number;
      initial: number;
    }
  | { kind: "checkbox"; id: string; label: string; initial: boolean }
  | { kind: "select"; id: string; label: string; options: string[]; initial: string }
  | { kind: "text"; id: string; label: string; initial: string }
  | {
      kind: "file";
      id: string;
      label: string;
      accept: string;
      onFile: (file: File | null) => void | Promise<void>;
    }
  | { kind: "button"; label: string; onClick: () => void };

export interface TestbedModule {
  id: string;
  name: string;
  init: (ctx: ModuleContext) => Promise<void> | void;
  ui?: (api: UiApi) => void;
  run: (job: RunRequest) => Promise<ModuleOutput[]>;
  dispose?: () => void;
}
