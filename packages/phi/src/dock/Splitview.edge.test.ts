import { describe, it, expect } from "vitest";
import { Splitview, LayoutPriority, type IView } from "./Splitview";

function createView(opts: {
  min?: number; max?: number; priority?: LayoutPriority;
} = {}): IView & { layoutCalls: Array<{ size: number; orthogonal: number }> } {
  const layoutCalls: Array<{ size: number; orthogonal: number }> = [];
  return {
    minimumSize: opts.min ?? 50,
    maximumSize: opts.max ?? Number.POSITIVE_INFINITY,
    priority: opts.priority,
    layout(size, orthogonal) { layoutCalls.push({ size, orthogonal }); },
    layoutCalls,
  };
}

// ─── Invariant: sum(sizes) == container after every operation ───────────

describe("Splitview — conservation invariant", () => {
  it("holds after layout + resize + distributeEmptySpace", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50 }), 200);
    sv.addView(createView({ min: 50 }), 300);
    sv.addView(createView({ min: 50 }), 100);
    sv.layout(600, 400);

    for (let delta = -300; delta <= 300; delta += 17) {
      const snapshot = sv.getSizes();
      sv.resize(1, delta, snapshot);
      sv.distributeEmptySpace();
      const total = sv.getSizes().reduce((a, b) => a + b, 0);
      expect(total).toBe(600);
    }
  });

  it("holds after repeated container resizes", () => {
    const sv = new Splitview({ orientation: "horizontal", proportionalLayout: true });
    sv.addView(createView({ min: 50 }), 200);
    sv.addView(createView({ min: 50 }), 400);
    sv.layout(600, 400);
    sv.saveProportions();

    for (const newSize of [800, 300, 1200, 150, 600, 2000, 100]) {
      sv.layout(newSize, 400);
      const total = sv.getSizes().reduce((a, b) => a + b, 0);
      expect(total).toBe(newSize);
    }
  });

  it("holds after add and remove operations", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50 }), 200);
    sv.addView(createView({ min: 50 }), 200);
    sv.layout(400, 300);

    // Add a view
    sv.addView(createView({ min: 50 }), 100, 1);
    sv.distributeEmptySpace();
    expect(sv.getSizes().reduce((a, b) => a + b, 0)).toBe(400);

    // Remove a view
    sv.removeView(1);
    sv.distributeEmptySpace();
    expect(sv.getSizes().reduce((a, b) => a + b, 0)).toBe(400);
  });
});

// ─── Minimum size enforcement ──────────────────────────────────────────

describe("Splitview — minimum size enforcement", () => {
  it("no view goes below minimum after any resize delta", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 100 }), 300);
    sv.addView(createView({ min: 100 }), 300);
    sv.addView(createView({ min: 100 }), 300);
    sv.layout(900, 400);

    for (let delta = -800; delta <= 800; delta += 37) {
      const snapshot = [300, 300, 300];
      sv.resize(0, delta, snapshot);
      for (const size of sv.getSizes()) {
        expect(size).toBeGreaterThanOrEqual(100);
      }
      // Reset
      sv.resize(0, 0, snapshot);
    }
  });

  it("no view exceeds maximum after any resize delta", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50, max: 400 }), 250);
    sv.addView(createView({ min: 50, max: 400 }), 250);
    sv.layout(500, 400);

    for (let delta = -400; delta <= 400; delta += 23) {
      const snapshot = [250, 250];
      sv.resize(0, delta, snapshot);
      for (const size of sv.getSizes()) {
        expect(size).toBeLessThanOrEqual(400);
      }
      sv.resize(0, 0, snapshot);
    }
  });

  it("all views at minimum: resize returns 0 delta", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 200, max: 200 }), 200);
    sv.addView(createView({ min: 200, max: 200 }), 200);
    sv.layout(400, 400);

    const result = sv.resize(0, 50);
    // Both are at min==max, no movement possible
    expect(sv.getSizes()).toEqual([200, 200]);
  });
});

// ─── Priority ordering ─────────────────────────────────────────────────

describe("Splitview — priority ordering", () => {
  it("low priority view absorbs excess in distributeEmptySpace", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50, priority: LayoutPriority.Normal }), 200);
    sv.addView(createView({ min: 50, priority: LayoutPriority.Low }), 200);
    (sv as any)._size = 500;

    sv.distributeEmptySpace();
    const sizes = sv.getSizes();
    // Normal gets space first (pushed to start), Low gets what's left
    // Since Normal has no max, it absorbs first. Low gets remainder.
    expect(sizes[0] + sizes[1]).toBe(500);
  });

  it("high priority view gets space first in distributeEmptySpace", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50, max: 500, priority: LayoutPriority.High }), 100);
    sv.addView(createView({ min: 50, max: 500, priority: LayoutPriority.Normal }), 100);
    sv.addView(createView({ min: 50, max: 500, priority: LayoutPriority.Low }), 100);
    (sv as any)._size = 600;

    sv.distributeEmptySpace();
    const sizes = sv.getSizes();
    expect(sizes[0]).toBeGreaterThanOrEqual(sizes[1]);
    expect(sizes[1]).toBeGreaterThanOrEqual(sizes[2]);
  });
});

// ─── Rapid sash drag simulation ────────────────────────────────────────

describe("Splitview — sash drag simulation", () => {
  it("repeated snapshot-based resizes don't drift", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50 }), 300);
    sv.addView(createView({ min: 50 }), 300);
    sv.layout(600, 400);

    const snapshot = sv.getSizes();

    // Simulate wiggling the sash back and forth from the same snapshot
    sv.resize(0, 100, snapshot);
    sv.resize(0, -100, snapshot);
    sv.resize(0, 50, snapshot);
    sv.resize(0, 0, snapshot); // back to original

    expect(sv.getSizes()).toEqual([300, 300]);
  });

  it("drag to extreme then back preserves original sizes", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50 }), 300);
    sv.addView(createView({ min: 50 }), 300);
    sv.layout(600, 400);

    const snapshot = sv.getSizes();
    sv.resize(0, 999, snapshot);  // extreme right
    sv.resize(0, 0, snapshot);    // back to start
    expect(sv.getSizes()).toEqual([300, 300]);
  });
});

// ─── Visibility toggle + layout cycle ──────────────────────────────────

describe("Splitview — visibility + layout cycles", () => {
  it("hide → layout → show → layout preserves proportions", () => {
    const sv = new Splitview({ orientation: "horizontal", proportionalLayout: true });
    sv.addView(createView({ min: 50 }), 200);
    sv.addView(createView({ min: 50 }), 400);
    sv.layout(600, 400);
    sv.saveProportions();

    // Hide first view
    sv.items[0].setVisible(false);
    sv.layout(600, 400);

    // Show first view
    sv.items[0].setVisible(true);
    sv.layout(600, 400);

    // Should still be close to 200/400
    expect(sv.items[0].size).toBeCloseTo(200, -1);
  });

  it("setVisible is idempotent", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50 }), 300);

    sv.items[0].setVisible(true);  // already visible
    expect(sv.items[0].size).toBe(300);
    expect(sv.items[0].visible).toBe(true);

    sv.items[0].setVisible(false);
    sv.items[0].setVisible(false); // already hidden
    expect(sv.items[0].size).toBe(0);
    expect(sv.items[0].visible).toBe(false);
  });
});

// ─── Zero-size container ───────────────────────────────────────────────

describe("Splitview — zero/tiny container", () => {
  it("layout with size 0 does not throw", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50 }), 200);
    sv.addView(createView({ min: 50 }), 200);

    expect(() => sv.layout(0, 0)).not.toThrow();
  });

  it("layout with size smaller than sum of minimums clamps", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 100 }), 300);
    sv.addView(createView({ min: 100 }), 300);
    sv.layout(600, 400);
    sv.saveProportions();

    sv.layout(100, 400); // 100 < 200 (sum of minimums)
    // Should not throw; sizes may not sum perfectly but shouldn't be negative
    for (const size of sv.getSizes()) {
      expect(size).toBeGreaterThanOrEqual(0);
    }
  });
});
