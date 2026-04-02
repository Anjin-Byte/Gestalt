import type { LatticeTopology, LatticeType } from "./types";

const cubicTopology = (): LatticeTopology => ({
  nodes: [
    [0, 0, 0],
    [1, 0, 0],
    [1, 1, 0],
    [0, 1, 0],
    [0, 0, 1],
    [1, 0, 1],
    [1, 1, 1],
    [0, 1, 1],
  ],
  edges: [
    [0, 1], [1, 2], [2, 3], [3, 0],
    [4, 5], [5, 6], [6, 7], [7, 4],
    [0, 4], [1, 5], [2, 6], [3, 7],
  ],
});

const bccTopology = (): LatticeTopology => ({
  nodes: [
    [0, 0, 0],
    [1, 0, 0],
    [1, 1, 0],
    [0, 1, 0],
    [0, 0, 1],
    [1, 0, 1],
    [1, 1, 1],
    [0, 1, 1],
    [0.5, 0.5, 0.5],
  ],
  edges: [
    [0, 1], [1, 2], [2, 3], [3, 0],
    [4, 5], [5, 6], [6, 7], [7, 4],
    [0, 4], [1, 5], [2, 6], [3, 7],
    [8, 0], [8, 1], [8, 2], [8, 3],
    [8, 4], [8, 5], [8, 6], [8, 7],
  ],
});

const kelvinTopology = (): LatticeTopology => {
  throw new Error("Kelvin lattice is not implemented in the legacy scaffold yet.");
};

export const getTopology = (kind: LatticeType): LatticeTopology => {
  switch (kind) {
    case "cubic":
      return cubicTopology();
    case "bcc":
      return bccTopology();
    case "kelvin":
      return kelvinTopology();
    default:
      return cubicTopology();
  }
};
