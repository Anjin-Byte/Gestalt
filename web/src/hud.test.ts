// @vitest-environment happy-dom
// Hud DOM-writer tests: exact cell text, including the compact count format
// (the native viewer's) and the em-dash placeholders for a stalled loop.
import { beforeEach, describe, expect, it } from "vitest";

import { Hud, type HudElements } from "./hud";
import { type HudStats, type SceneMeta } from "./render-protocol";

let el: HudElements;
let hud: Hud;

beforeEach(() => {
  el = {
    fps: document.createElement("dd"),
    frame: document.createElement("dd"),
    frames: document.createElement("dd"),
    nodes: document.createElement("dd"),
    leaves: document.createElement("dd"),
    voxels: document.createElement("dd"),
    res: document.createElement("dd"),
    heap: document.createElement("dd"),
  };
  hud = new Hud(el);
});

function stats(overrides: Partial<HudStats>): HudStats {
  return {
    fps: 60,
    frameAvg: 16.6,
    frameMin: 15,
    frameMax: 18,
    frames: 100,
    nodes: 0,
    leaves: 0,
    voxels: 0,
    undoDepth: 0,
    redoDepth: 0,
    truecolor: false,
    heapBytes: 0,
    ...overrides,
  };
}

describe("Hud.setStats", () => {
  it("writes rounded fps and the avg (min–max) frame time", () => {
    hud.setStats(stats({ fps: 119.9, frameAvg: 8.34, frameMin: 7.96, frameMax: 12.04 }));
    expect(el.fps.textContent).toBe("120");
    expect(el.frame.textContent).toBe("8.3 (8.0–12.0) ms");
  });

  it("shows em-dashes before the first frame instead of 0/NaN", () => {
    hud.setStats(stats({ fps: 0, frameAvg: 0, frameMin: 0, frameMax: 0 }));
    expect(el.fps.textContent).toBe("—");
    expect(el.frame.textContent).toBe("—");
  });

  it("formats counters compactly, exactly like the native viewer", () => {
    hud.setStats(stats({ frames: 999, nodes: 1000, leaves: 142_078, voxels: 5_585_909 }));
    expect(el.frames.textContent).toBe("999");
    expect(el.nodes.textContent).toBe("1.0K");
    expect(el.leaves.textContent).toBe("142.1K");
    expect(el.voxels.textContent).toBe("5.59M");
  });
});

describe("Hud heap gauge", () => {
  it("shows em-dashes until each side reports", () => {
    hud.setIoHeap(96 * 2 ** 20);
    expect(el.heap.textContent).toBe("r — · io 96M");
  });

  it("writes both gauges: mebibytes small, gibibytes past 1 GiB", () => {
    hud.setStats(stats({ heapBytes: 212 * 2 ** 20 }));
    hud.setIoHeap(1.25 * 2 ** 30);
    expect(el.heap.textContent).toBe("r 212M · io 1.25G");
  });

  it("shows a recycled IO worker as freed (0M), not as unknown", () => {
    hud.setStats(stats({ heapBytes: 64 * 2 ** 20 }));
    hud.setIoHeap(0);
    expect(el.heap.textContent).toBe("r 64M · io 0M");
  });
});

describe("Hud.setScene", () => {
  it("writes the scene identity cells", () => {
    const scene: SceneMeta = {
      label: "littlest-tokyo.glb",
      nodes: 3,
      leaves: 40,
      voxels: 133_252,
      res: 128,
      editable: false,
      truecolor: false,
    };
    hud.setScene(scene);
    expect(el.nodes.textContent).toBe("3");
    expect(el.leaves.textContent).toBe("40");
    expect(el.voxels.textContent).toBe("133.3K");
    expect(el.res.textContent).toBe("128³");
  });
});
