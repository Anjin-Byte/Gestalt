import { describe, it, expect, vi, beforeEach } from "vitest";
import { Splitview, LayoutPriority, type IView } from "./Splitview";

// ─── Test View Factory ─────────────────────────────────────────────────────

function createView(opts: {
  min?: number;
  max?: number;
  priority?: LayoutPriority;
  snap?: boolean;
} = {}): IView & { layoutCalls: Array<{ size: number; orthogonal: number }> } {
  const layoutCalls: Array<{ size: number; orthogonal: number }> = [];
  return {
    minimumSize: opts.min ?? 50,
    maximumSize: opts.max ?? Number.POSITIVE_INFINITY,
    priority: opts.priority,
    snap: opts.snap,
    layout(size, orthogonal) { layoutCalls.push({ size, orthogonal }); },
    layoutCalls,
  };
}

// ─── Construction ──────────────────────────────────────────────────────────

describe("Splitview — construction", () => {
  it("starts empty", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    expect(sv.length).toBe(0);
    expect(sv.size).toBe(0);
  });

  it("addView increases length", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView(), 200);
    sv.addView(createView(), 300);
    expect(sv.length).toBe(2);
  });

  it("removeView decreases length and returns the view", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    const v = createView();
    sv.addView(v, 200);
    const removed = sv.removeView(0);
    expect(removed).toBe(v);
    expect(sv.length).toBe(0);
  });

  it("getSizes returns current sizes", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView(), 200);
    sv.addView(createView(), 300);
    expect(sv.getSizes()).toEqual([200, 300]);
  });

  it("addView at specific index", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    const a = createView();
    const b = createView();
    const c = createView();
    sv.addView(a, 100);
    sv.addView(b, 100);
    sv.addView(c, 100, 1); // insert at index 1
    expect(sv.items[0].view).toBe(a);
    expect(sv.items[1].view).toBe(c);
    expect(sv.items[2].view).toBe(b);
  });
});

// ─── Resize (Constraint Solver) ────────────────────────────────────────────

describe("Splitview — resize (constraint solver)", () => {
  it("basic resize: grows one side, shrinks the other", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50 }), 300);
    sv.addView(createView({ min: 50 }), 300);

    sv.resize(0, 50); // grow left by 50, shrink right by 50
    expect(sv.getSizes()).toEqual([350, 250]);
  });

  it("resize respects minimum size on the shrinking side", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50 }), 300);
    sv.addView(createView({ min: 200 }), 300);

    // Try to grow left by 200 — right can only shrink to 200
    sv.resize(0, 200);
    expect(sv.getSizes()[1]).toBe(200); // clamped to minimum
    expect(sv.getSizes()[0]).toBe(400); // got what was available
  });

  it("resize respects maximum size on the growing side", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50, max: 350 }), 300);
    sv.addView(createView({ min: 50 }), 300);

    sv.resize(0, 100); // try to grow left to 400, but max is 350
    expect(sv.getSizes()[0]).toBe(350);
    expect(sv.getSizes()[1]).toBe(250);
  });

  it("negative delta shrinks the up-side", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50 }), 300);
    sv.addView(createView({ min: 50 }), 300);

    sv.resize(0, -100); // shrink left, grow right
    expect(sv.getSizes()).toEqual([200, 400]);
  });

  it("three views: resize at sash 1 affects views 0-1 and 2", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50 }), 200);
    sv.addView(createView({ min: 50 }), 200);
    sv.addView(createView({ min: 50 }), 200);

    sv.resize(1, 50); // grow views 0-1 side, shrink view 2
    const sizes = sv.getSizes();
    expect(sizes[0] + sizes[1]).toBe(450);
    expect(sizes[2]).toBe(150);
  });

  it("snapshot-based resize: delta relative to provided snapshot", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50 }), 300);
    sv.addView(createView({ min: 50 }), 300);

    const snapshot = sv.getSizes();
    sv.resize(0, 50, snapshot); // from snapshot: 300+50=350, 300-50=250
    expect(sv.getSizes()).toEqual([350, 250]);

    // Second resize from SAME snapshot (not incremental)
    sv.resize(0, 100, snapshot); // from snapshot: 300+100=400, 300-100=200
    expect(sv.getSizes()).toEqual([400, 200]);
  });

  it("delta clamped when both sides constrained", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 280, max: 320 }), 300);
    sv.addView(createView({ min: 280, max: 320 }), 300);

    sv.resize(0, 500); // massively over-request
    expect(sv.getSizes()[0]).toBeLessThanOrEqual(320);
    expect(sv.getSizes()[1]).toBeGreaterThanOrEqual(280);
  });

  it("conservation: sum of sizes doesn't change after resize", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50 }), 300);
    sv.addView(createView({ min: 50 }), 200);
    sv.addView(createView({ min: 50 }), 100);
    const totalBefore = sv.getSizes().reduce((a, b) => a + b, 0);

    sv.resize(1, 75);
    const totalAfter = sv.getSizes().reduce((a, b) => a + b, 0);
    expect(totalAfter).toBe(totalBefore);
  });
});

// ─── distributeEmptySpace ──────────────────────────────────────────────────

describe("Splitview — distributeEmptySpace", () => {
  it("fills leftover space into views", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50 }), 200);
    sv.addView(createView({ min: 50 }), 200);
    // Manually set container size larger than content
    (sv as any)._size = 500;

    sv.distributeEmptySpace();
    const total = sv.getSizes().reduce((a, b) => a + b, 0);
    expect(total).toBe(500);
  });

  it("removes excess space from views", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50 }), 300);
    sv.addView(createView({ min: 50 }), 300);
    (sv as any)._size = 400; // container smaller than content

    sv.distributeEmptySpace();
    const total = sv.getSizes().reduce((a, b) => a + b, 0);
    expect(total).toBe(400);
  });

  it("respects min/max during distribution", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 150, max: 200 }), 180);
    sv.addView(createView({ min: 50 }), 120);
    (sv as any)._size = 500;

    sv.distributeEmptySpace();
    expect(sv.getSizes()[0]).toBeLessThanOrEqual(200);
    expect(sv.getSizes()[0]).toBeGreaterThanOrEqual(150);
    const total = sv.getSizes().reduce((a, b) => a + b, 0);
    expect(total).toBe(500);
  });

  it("high priority views get space first", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50, priority: LayoutPriority.High }), 100);
    sv.addView(createView({ min: 50, priority: LayoutPriority.Low }), 100);
    (sv as any)._size = 400;

    sv.distributeEmptySpace();
    // High priority should have gotten more
    expect(sv.getSizes()[0]).toBeGreaterThan(sv.getSizes()[1]);
  });
});

// ─── Layout (Container Resize) ─────────────────────────────────────────────

describe("Splitview — layout (container resize)", () => {
  it("proportional resize preserves ratios", () => {
    const sv = new Splitview({ orientation: "horizontal", proportionalLayout: true });
    sv.addView(createView({ min: 50 }), 200);
    sv.addView(createView({ min: 50 }), 600);
    sv.layout(800, 400); // initial layout
    sv.saveProportions(); // save 0.25 / 0.75

    // Resize to 1200
    sv.layout(1200, 400);
    const sizes = sv.getSizes();
    // Should be approximately 300 / 900 (25/75)
    expect(sizes[0]).toBeCloseTo(300, -1);
    expect(sizes[1]).toBeCloseTo(900, -1);
  });

  it("proportional resize clamps to minimum", () => {
    const sv = new Splitview({ orientation: "horizontal", proportionalLayout: true });
    sv.addView(createView({ min: 100 }), 400);
    sv.addView(createView({ min: 100 }), 400);
    sv.layout(800, 400);
    sv.saveProportions();

    // Shrink drastically — can't go below 100 each
    sv.layout(200, 400);
    const sizes = sv.getSizes();
    expect(sizes[0]).toBeGreaterThanOrEqual(100);
    expect(sizes[1]).toBeGreaterThanOrEqual(100);
  });

  it("non-proportional layout uses resize at last sash", () => {
    const sv = new Splitview({ orientation: "horizontal", proportionalLayout: false });
    sv.addView(createView({ min: 50 }), 300);
    sv.addView(createView({ min: 50 }), 300);
    sv.layout(600, 400);

    // Grow container by 200 — should be absorbed by last view
    sv.layout(800, 400);
    const sizes = sv.getSizes();
    expect(sizes[0] + sizes[1]).toBe(800);
  });

  it("layout calls view.layout() for each view", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    const a = createView();
    const b = createView();
    sv.addView(a, 300);
    sv.addView(b, 300);

    sv.layout(600, 400);
    expect(a.layoutCalls.length).toBeGreaterThan(0);
    expect(b.layoutCalls.length).toBeGreaterThan(0);
    expect(a.layoutCalls.at(-1)?.orthogonal).toBe(400);
  });
});

// ─── Proportions ───────────────────────────────────────────────────────────

describe("Splitview — saveProportions", () => {
  it("saves and restores proportions", () => {
    const sv = new Splitview({ orientation: "horizontal", proportionalLayout: true });
    sv.addView(createView({ min: 50 }), 250);
    sv.addView(createView({ min: 50 }), 750);
    sv.layout(1000, 400);
    sv.saveProportions();

    // Resize to 2000 — should double
    sv.layout(2000, 400);
    expect(sv.getSizes()[0]).toBeCloseTo(500, -1);
    expect(sv.getSizes()[1]).toBeCloseTo(1500, -1);
  });

  it("proportions not saved when proportionalLayout=false", () => {
    const sv = new Splitview({ orientation: "horizontal", proportionalLayout: false });
    sv.addView(createView(), 300);
    sv.addView(createView(), 300);
    sv.layout(600, 400);
    sv.saveProportions(); // should be a no-op

    // Growing should push to last view, not proportional
    sv.layout(900, 400);
    // First view should stay near 300 (not grow to 450)
    expect(sv.getSizes()[0]).toBe(300);
    expect(sv.getSizes()[1]).toBe(600);
  });
});

// ─── Visibility ────────────────────────────────────────────────────────────

describe("Splitview — view visibility", () => {
  it("hidden view has size 0", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView(), 300);
    sv.addView(createView(), 300);

    sv.items[0].setVisible(false);
    expect(sv.items[0].size).toBe(0);
    expect(sv.items[0].visible).toBe(false);
  });

  it("showing a hidden view restores cached size", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50, max: 500 }), 300);
    sv.addView(createView(), 300);

    sv.items[0].setVisible(false);
    expect(sv.items[0].size).toBe(0);

    sv.items[0].setVisible(true);
    expect(sv.items[0].size).toBe(300);
  });

  it("hidden view min/max are 0", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 100, max: 500 }), 300);

    sv.items[0].setVisible(false);
    expect(sv.items[0].minimumSize).toBe(0);
    expect(sv.items[0].maximumSize).toBe(0);
  });
});

// ─── Edge Cases ────────────────────────────────────────────────────────────

describe("Splitview — edge cases", () => {
  it("single view fills container after layout", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50 }), 100);
    sv.layout(800, 400);
    expect(sv.getSizes()).toEqual([800]);
  });

  it("resize with invalid index returns 0", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView(), 300);
    const result = sv.resize(-1, 50);
    expect(result).toBe(0);
  });

  it("resize with no down-group (last sash) works", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    sv.addView(createView({ min: 50 }), 300);
    sv.addView(createView({ min: 50 }), 300);

    // Resize at last valid sash index
    sv.resize(1, 50);
    // No down-group, so only up-group changes — effectively a no-constraint grow
    const total = sv.getSizes().reduce((a, b) => a + b, 0);
    // Total might exceed original since there's nothing to take from
    expect(total).toBeGreaterThanOrEqual(600);
  });

  it("many views: resize propagates correctly", () => {
    const sv = new Splitview({ orientation: "horizontal" });
    for (let i = 0; i < 5; i++) {
      sv.addView(createView({ min: 50 }), 200);
    }
    const totalBefore = sv.getSizes().reduce((a, b) => a + b, 0);

    sv.resize(2, 100); // resize at middle sash
    const totalAfter = sv.getSizes().reduce((a, b) => a + b, 0);
    expect(totalAfter).toBe(totalBefore);

    // All sizes should be >= minimum
    for (const size of sv.getSizes()) {
      expect(size).toBeGreaterThanOrEqual(50);
    }
  });
});
