export type LatticeType = "cubic" | "bcc" | "kelvin";
export type OriginMode = "centered" | "grid-origin";

export type LatticeNode = [number, number, number];
export type LatticeEdge = readonly [number, number];

export type LatticeTopology = {
  nodes: LatticeNode[];
  edges: LatticeEdge[];
};

export type LatticeParams = {
  latticeType: LatticeType;
  cellSize: number;
  strutRadius: number;
  originMode: OriginMode;
  latticeOrigin: [number, number, number];
};

export type LatticeRunParams = {
  gridDim: number;
  voxelSize: number;
  epsilon: number;
  fitBounds: boolean;
  debugChunkBounds: boolean;
  debugWireframe: boolean;
  colorMode: "none" | "material" | "chunk" | "face-direction" | "quad-size";
  showHostVoxels: boolean;
  showResultVoxels: boolean;
  lattice: LatticeParams;
};
