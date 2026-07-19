import * as fc from "fast-check";
import { describe, expect, it, vi } from "vitest";

import { FrameRing, takeScene, type KernelCounts } from "./render-protocol";

const COUNTS: KernelCounts = {
  frames: 7,
  nodes: 11,
  leaves: 13,
  voxels: 17,
  undoDepth: 2,
  redoDepth: 1,
  truecolor: false,
};

describe("FrameRing", () => {
  it("reports zeroed timings before any sample, passing counters through", () => {
    const ring = new FrameRing();
    const stats = ring.stats(COUNTS);
    expect(stats.fps).toBe(0);
    expect(stats.frameAvg).toBe(0);
    expect(stats.frameMin).toBe(0);
    expect(stats.frameMax).toBe(0);
    expect(stats.frames).toBe(7);
    expect(stats.voxels).toBe(17);
    expect(stats.undoDepth).toBe(2);
    expect(stats.redoDepth).toBe(1);
  });

  it("aggregates min/avg/max and derives fps from the average", () => {
    const ring = new FrameRing();
    for (const dt of [10, 20, 30]) {
      ring.sample(dt);
    }
    const stats = ring.stats(COUNTS);
    expect(stats.frameMin).toBe(10);
    expect(stats.frameMax).toBe(30);
    expect(stats.frameAvg).toBe(20);
    expect(stats.fps).toBeCloseTo(50);
  });

  it("keeps a bounded window: old samples fall out after wraparound", () => {
    const ring = new FrameRing();
    ring.sample(1000); // the outlier that must age out
    for (let i = 0; i < 120; i += 1) {
      ring.sample(10);
    }
    expect(ring.stats(COUNTS).frameMax).toBe(10);
  });

  it("reset forgets the window (scene changes restart the sample)", () => {
    const ring = new FrameRing();
    ring.sample(25);
    ring.reset();
    expect(ring.stats(COUNTS).frameAvg).toBe(0);
  });

  // Property: for any sample stream, the aggregate matches a straightforward
  // reference over the last 120 samples (differential oracle; the ring stores
  // f32, so the reference rounds through Math.fround).
  it("matches a reference aggregation across arbitrary sample streams", () => {
    fc.assert(
      fc.property(
        fc.array(fc.double({ min: 0.01, max: 10_000, noNaN: true }), { maxLength: 300 }),
        (dts) => {
          const ring = new FrameRing();
          for (const dt of dts) {
            ring.sample(dt);
          }
          const stats = ring.stats(COUNTS);
          const window = dts.slice(-120).map(Math.fround);
          if (window.length === 0) {
            expect(stats).toMatchObject({ fps: 0, frameAvg: 0, frameMin: 0, frameMax: 0 });
            return;
          }
          const avg = window.reduce((a, b) => a + b, 0) / window.length;
          expect(stats.frameMin).toBe(Math.min(...window));
          expect(stats.frameMax).toBe(Math.max(...window));
          expect(stats.frameAvg).toBeCloseTo(avg, 4);
          expect(stats.fps).toBeCloseTo(1000 / avg, 4);
        },
      ),
      { seed: 42, numRuns: 200 },
    );
  });
});

describe("takeScene", () => {
  it("copies every field into plain data and frees the wasm object once", () => {
    const free = vi.fn();
    const scene = takeScene({
      label: "tokyo.glb",
      nodes: 100,
      leaves: 200,
      voxels: 300,
      res: 512,
      editable: false,
      truecolor: true,
      free,
    });
    expect(scene).toEqual({
      label: "tokyo.glb",
      nodes: 100,
      leaves: 200,
      voxels: 300,
      res: 512,
      editable: false,
      truecolor: true,
    });
    expect(free).toHaveBeenCalledTimes(1);
  });
});
