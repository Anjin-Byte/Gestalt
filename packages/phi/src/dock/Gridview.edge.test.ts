import { describe, it, expect, beforeEach } from "vitest";
import { Gridview, LeafNode, BranchNode, type IGridView } from "./Gridview";

let nextId = 0;
function view(opts: { id?: string; minW?: number; minH?: number } = {}): IGridView & { id: string } {
  const id = opts.id ?? `v-${nextId++}`;
  return {
    id,
    minimumWidth: opts.minW ?? 100,
    maximumWidth: Number.POSITIVE_INFINITY,
    minimumHeight: opts.minH ?? 50,
    maximumHeight: Number.POSITIVE_INFINITY,
    layout() {},
  };
}
beforeEach(() => { nextId = 0; });

// ─── Tree structure invariants ─────────────────────────────────────────

describe("Gridview — tree invariants", () => {
  it("root is always a BranchNode", () => {
    const grid = new Gridview("horizontal");
    expect(grid.root.kind).toBe("branch");

    grid.addView(view({ id: "A" }), 400, [0]);
    expect(grid.root.kind).toBe("branch");

    grid.removeView([0]);
    expect(grid.root.kind).toBe("branch");
  });

  it("no single-child branches exist after removal", () => {
    const grid = new Gridview("horizontal");
    grid.addView(view({ id: "A" }), 300, [0]);
    grid.addView(view({ id: "B" }), 300, [1]);
    grid.addView(view({ id: "C" }), 200, [1, 1]); // creates branch at [1]

    // Remove B — branch at [1] has only C left → should collapse
    grid.removeView([1, 0]);
    // root should have 2 leaves (A, C), no branches
    expect(grid.root.children.every((c) => c.kind === "leaf")).toBe(true);
    expect(grid.root.children).toHaveLength(2);
  });

  it("alternating orientation: depth 0 matches root, depth 1 is orthogonal", () => {
    const grid = new Gridview("horizontal");
    grid.addView(view({ id: "A" }), 400, [0]);
    grid.addView(view({ id: "B" }), 400, [1]);
    grid.addViewAt(view({ id: "C" }), 200, "down", [1]); // creates vertical branch

    expect(grid.root.orientation).toBe("horizontal");
    const branch = grid.root.children[1] as BranchNode;
    expect(branch.orientation).toBe("vertical");
  });

  it("deep nesting: 3 levels of alternating orientation", () => {
    const grid = new Gridview("horizontal");
    grid.addView(view({ id: "A" }), 500, [0]);
    grid.addViewAt(view({ id: "B" }), 250, "down", [0]);   // vertical branch at [0]
    grid.addViewAt(view({ id: "C" }), 125, "right", [0, 1]); // horizontal branch at [0][1]

    // Root: horizontal
    expect(grid.root.orientation).toBe("horizontal");
    // [0]: vertical branch
    const level1 = grid.root.children[0] as BranchNode;
    expect(level1.kind).toBe("branch");
    expect(level1.orientation).toBe("vertical");
    // [0][1]: horizontal branch
    const level2 = level1.children[1] as BranchNode;
    expect(level2.kind).toBe("branch");
    expect(level2.orientation).toBe("horizontal");
  });
});

// ─── Removal edge cases ────────────────────────────────────────────────

describe("Gridview — removal edge cases", () => {
  it("remove the only view leaves root with 0 children", () => {
    const grid = new Gridview("horizontal");
    grid.addView(view({ id: "A" }), 400, [0]);
    grid.removeView([0]);
    expect(grid.root.children).toHaveLength(0);
  });

  it("remove from deeply nested branch collapses recursively", () => {
    const grid = new Gridview("horizontal");
    grid.addView(view({ id: "A" }), 300, [0]);
    grid.addView(view({ id: "B" }), 500, [1]);
    grid.addViewAt(view({ id: "C" }), 250, "down", [1]);    // [1] → branch {B, C}
    grid.addViewAt(view({ id: "D" }), 125, "right", [1, 1]); // [1][1] → branch {C, D}

    // Tree: A | branch( B | branch(C | D) )
    expect(grid.root.children).toHaveLength(2);
    const mid = grid.root.children[1] as BranchNode;
    expect(mid.children[1].kind).toBe("branch");

    // Remove D — inner branch collapses
    grid.removeView([1, 1, 1]);
    const mid2 = grid.root.children[1] as BranchNode;
    expect(mid2.children[1].kind).toBe("leaf");
    expect((mid2.children[1] as LeafNode).view).toHaveProperty("id", "C");
  });

  it("remove last view from a branch within root promotes sibling", () => {
    const grid = new Gridview("horizontal");
    grid.addView(view({ id: "A" }), 400, [0]);
    grid.addView(view({ id: "B" }), 400, [1]);
    grid.addViewAt(view({ id: "C" }), 200, "down", [0]); // [0] → branch {A, C}

    // Remove A — branch has only C → collapses to leaf
    grid.removeView([0, 0]);
    expect(grid.root.children[0].kind).toBe("leaf");
    expect((grid.root.children[0] as LeafNode).view).toHaveProperty("id", "C");
  });
});

// ─── addViewAt all 4 directions ────────────────────────────────────────

describe("Gridview — all 4 directions from same target", () => {
  it("can add views in all 4 directions around a center view", () => {
    const grid = new Gridview("horizontal");
    grid.addView(view({ id: "center" }), 400, [0]);

    grid.addViewAt(view({ id: "left" }), 150, "left", [0]);
    grid.addViewAt(view({ id: "right" }), 150, "right", [1]); // center is now at [1]
    grid.addViewAt(view({ id: "top" }), 100, "up", [1]);       // center is now at [1][1]
    grid.addViewAt(view({ id: "bottom" }), 100, "down", [1, 1]); // center is [1][1][0] after restructuring... this gets complex

    // Just verify no errors and all views are findable
    grid.layout(800, 600);

    function collectIds(node: any): string[] {
      if (node.kind === "leaf") return [(node.view as any).id];
      return node.children.flatMap(collectIds);
    }
    const ids = collectIds(grid.root);
    expect(ids).toContain("center");
    expect(ids).toContain("left");
    expect(ids).toContain("right");
    expect(ids).toContain("top");
    expect(ids).toContain("bottom");
    expect(ids).toHaveLength(5);
  });
});

// ─── Serialization edge cases ──────────────────────────────────────────

describe("Gridview — serialization edge cases", () => {
  it("serialize empty grid", () => {
    const grid = new Gridview("horizontal");
    grid.layout(800, 600);
    const data = grid.serialize();
    expect(data.root.type).toBe("branch");
    expect((data.root as any).data).toHaveLength(0);
  });

  it("serialize single view", () => {
    const grid = new Gridview("horizontal");
    grid.addView(view({ id: "solo" }), 800, [0]);
    grid.layout(800, 600);
    const data = grid.serialize();
    expect((data.root as any).data).toHaveLength(1);
    expect((data.root as any).data[0].type).toBe("leaf");
    expect((data.root as any).data[0].data).toBe("solo");
  });

  it("roundtrip preserves deeply nested structure", () => {
    const grid = new Gridview("horizontal");
    grid.addView(view({ id: "A" }), 300, [0]);
    grid.addView(view({ id: "B" }), 500, [1]);
    grid.addViewAt(view({ id: "C" }), 250, "down", [1]);
    grid.addViewAt(view({ id: "D" }), 125, "right", [1, 1]);
    grid.layout(800, 600);

    const data = grid.serialize();
    const restored = Gridview.deserialize(data, (id) => view({ id }));

    function collectIds(node: any): string[] {
      if (node.kind === "leaf") return [(node.view as any).id];
      return node.children.flatMap(collectIds);
    }
    const original = collectIds(grid.root).sort();
    const restoredIds = collectIds(restored.root).sort();
    expect(restoredIds).toEqual(original);
  });

  it("roundtrip with vertical root orientation", () => {
    const grid = new Gridview("vertical");
    grid.addView(view({ id: "top" }), 300, [0]);
    grid.addView(view({ id: "bottom" }), 300, [1]);
    grid.layout(800, 600);

    const data = grid.serialize();
    expect(data.orientation).toBe("vertical");

    const restored = Gridview.deserialize(data, (id) => view({ id }));
    expect(restored.orientation).toBe("vertical");
    expect(restored.root.children).toHaveLength(2);
  });
});

// ─── Layout propagation ────────────────────────────────────────────────

describe("Gridview — layout propagation", () => {
  it("all leaves receive layout calls after grid.layout()", () => {
    const views: (IGridView & { id: string; lastLayout?: { w: number; h: number } })[] = [];
    function trackedView(id: string) {
      const v = {
        ...view({ id }),
        lastLayout: undefined as { w: number; h: number } | undefined,
        layout(w: number, h: number) { this.lastLayout = { w, h }; },
      };
      views.push(v);
      return v;
    }

    const grid = new Gridview("horizontal");
    grid.addView(trackedView("A"), 300, [0]);
    grid.addView(trackedView("B"), 500, [1]);
    grid.addViewAt(trackedView("C"), 200, "down", [1]);

    grid.layout(800, 600);

    for (const v of views) {
      expect(v.lastLayout).toBeDefined();
      expect(v.lastLayout!.w).toBeGreaterThan(0);
      expect(v.lastLayout!.h).toBeGreaterThan(0);
    }
  });

  it("leaf dimensions sum correctly in each branch", () => {
    const views: (IGridView & { id: string; lastLayout?: { w: number; h: number } })[] = [];
    function trackedView(id: string) {
      const v = {
        ...view({ id }),
        lastLayout: undefined as { w: number; h: number } | undefined,
        layout(w: number, h: number) { this.lastLayout = { w, h }; },
      };
      views.push(v);
      return v;
    }

    const grid = new Gridview("horizontal");
    grid.addView(trackedView("A"), 300, [0]);
    grid.addView(trackedView("B"), 500, [1]);
    grid.layout(800, 600);

    const a = views.find((v) => v.id === "A")!;
    const b = views.find((v) => v.id === "B")!;
    // Widths should sum to total (horizontal root)
    expect(a.lastLayout!.w + b.lastLayout!.w).toBe(800);
    // Heights should equal container height
    expect(a.lastLayout!.h).toBe(600);
    expect(b.lastLayout!.h).toBe(600);
  });
});

// ─── Stress test ───────────────────────────────────────────────────────

describe("Gridview — stress", () => {
  it("add 20 views in alternating directions without error", () => {
    const grid = new Gridview("horizontal");
    grid.addView(view({ id: "root" }), 400, [0]);

    const directions: ("left" | "right" | "up" | "down")[] = ["right", "down", "left", "up"];
    for (let i = 0; i < 19; i++) {
      const dir = directions[i % 4];
      // Always target the first leaf we can find
      grid.addViewAt(view({ id: `v${i}` }), 50, dir, [0]);
    }

    grid.layout(1200, 800);

    function countLeaves(node: any): number {
      if (node.kind === "leaf") return 1;
      return node.children.reduce((s: number, c: any) => s + countLeaves(c), 0);
    }
    expect(countLeaves(grid.root)).toBe(20);
  });
});
