import { describe, it, expect, beforeEach } from "vitest";
import { Gridview, LeafNode, BranchNode, type IGridView, type SerializedGridview } from "./Gridview";

// ─── Test View Factory ─────────────────────────────────────────────────────

let nextId = 0;

function createGridView(opts: {
  id?: string;
  minW?: number;
  maxW?: number;
  minH?: number;
  maxH?: number;
} = {}): IGridView & { id: string; lastLayout?: { w: number; h: number } } {
  const id = opts.id ?? `view-${nextId++}`;
  return {
    id,
    minimumWidth: opts.minW ?? 100,
    maximumWidth: opts.maxW ?? Number.POSITIVE_INFINITY,
    minimumHeight: opts.minH ?? 50,
    maximumHeight: opts.maxH ?? Number.POSITIVE_INFINITY,
    lastLayout: undefined,
    layout(w, h) { this.lastLayout = { w, h }; },
  };
}

beforeEach(() => { nextId = 0; });

// ─── Basic Construction ────────────────────────────────────────────────────

describe("Gridview — construction", () => {
  it("creates with a root branch", () => {
    const grid = new Gridview("horizontal");
    expect(grid.root.kind).toBe("branch");
    expect(grid.root.children).toHaveLength(0);
    expect(grid.orientation).toBe("horizontal");
  });

  it("can change orientation", () => {
    const gridH = new Gridview("horizontal");
    const gridV = new Gridview("vertical");
    expect(gridH.orientation).toBe("horizontal");
    expect(gridV.orientation).toBe("vertical");
  });
});

// ─── Adding Views ──────────────────────────────────────────────────────────

describe("Gridview — addView", () => {
  it("adds a single view at root level", () => {
    const grid = new Gridview("horizontal");
    const view = createGridView();
    grid.addView(view, 400, [0]);
    expect(grid.root.children).toHaveLength(1);
    expect(grid.root.children[0].kind).toBe("leaf");
  });

  it("adds two views side by side", () => {
    const grid = new Gridview("horizontal");
    grid.addView(createGridView({ id: "A" }), 400, [0]);
    grid.addView(createGridView({ id: "B" }), 200, [1]);

    expect(grid.root.children).toHaveLength(2);
    expect((grid.root.children[0] as LeafNode).view).toHaveProperty("id", "A");
    expect((grid.root.children[1] as LeafNode).view).toHaveProperty("id", "B");
  });

  it("inserts at specific index", () => {
    const grid = new Gridview("horizontal");
    grid.addView(createGridView({ id: "A" }), 300, [0]);
    grid.addView(createGridView({ id: "C" }), 300, [1]);
    grid.addView(createGridView({ id: "B" }), 200, [1]); // insert between A and C

    expect(grid.root.children).toHaveLength(3);
    expect((grid.root.children[1] as LeafNode).view).toHaveProperty("id", "B");
  });

  it("tree restructuring: adding a deeper level creates a branch", () => {
    const grid = new Gridview("horizontal");
    grid.addView(createGridView({ id: "A" }), 400, [0]);
    grid.addView(createGridView({ id: "B" }), 400, [1]);

    // Add a view below B — this should restructure: B is replaced by a branch containing B + C
    grid.addView(createGridView({ id: "C" }), 200, [1, 1]);

    expect(grid.root.children).toHaveLength(2);
    expect(grid.root.children[0].kind).toBe("leaf"); // A unchanged
    expect(grid.root.children[1].kind).toBe("branch"); // new branch

    const branch = grid.root.children[1] as BranchNode;
    expect(branch.children).toHaveLength(2);
    expect((branch.children[0] as LeafNode).view).toHaveProperty("id", "B");
    expect((branch.children[1] as LeafNode).view).toHaveProperty("id", "C");
  });
});

// ─── addViewAt (Direction-Based) ───────────────────────────────────────────

describe("Gridview — addViewAt (direction-based)", () => {
  it("add to the right of a view at the same level", () => {
    const grid = new Gridview("horizontal");
    grid.addView(createGridView({ id: "A" }), 400, [0]);

    // Add B to the right of A
    grid.addViewAt(createGridView({ id: "B" }), 200, "right", [0]);

    expect(grid.root.children).toHaveLength(2);
    expect((grid.root.children[0] as LeafNode).view).toHaveProperty("id", "A");
    expect((grid.root.children[1] as LeafNode).view).toHaveProperty("id", "B");
  });

  it("add to the left of a view at the same level", () => {
    const grid = new Gridview("horizontal");
    grid.addView(createGridView({ id: "A" }), 400, [0]);

    grid.addViewAt(createGridView({ id: "B" }), 200, "left", [0]);

    expect(grid.root.children).toHaveLength(2);
    expect((grid.root.children[0] as LeafNode).view).toHaveProperty("id", "B");
    expect((grid.root.children[1] as LeafNode).view).toHaveProperty("id", "A");
  });

  it("add below a view creates a deeper branch (orthogonal direction)", () => {
    const grid = new Gridview("horizontal");
    grid.addView(createGridView({ id: "A" }), 400, [0]);
    grid.addView(createGridView({ id: "B" }), 400, [1]);

    // Add C below B — orthogonal to root's horizontal orientation
    grid.addViewAt(createGridView({ id: "C" }), 200, "down", [1]);

    expect(grid.root.children).toHaveLength(2);
    expect(grid.root.children[1].kind).toBe("branch");

    const branch = grid.root.children[1] as BranchNode;
    expect(branch.children).toHaveLength(2);
    expect((branch.children[0] as LeafNode).view).toHaveProperty("id", "B");
    expect((branch.children[1] as LeafNode).view).toHaveProperty("id", "C");
  });

  it("add above a view inserts at index 0 in the new branch", () => {
    const grid = new Gridview("horizontal");
    grid.addView(createGridView({ id: "A" }), 400, [0]);

    grid.addViewAt(createGridView({ id: "B" }), 200, "up", [0]);

    // Should restructure: root[0] = branch { B, A }
    expect(grid.root.children[0].kind).toBe("branch");
    const branch = grid.root.children[0] as BranchNode;
    expect((branch.children[0] as LeafNode).view).toHaveProperty("id", "B");
    expect((branch.children[1] as LeafNode).view).toHaveProperty("id", "A");
  });
});

// ─── Removing Views ────────────────────────────────────────────────────────

describe("Gridview — removeView", () => {
  it("removes a leaf from root level", () => {
    const grid = new Gridview("horizontal");
    grid.addView(createGridView({ id: "A" }), 400, [0]);
    grid.addView(createGridView({ id: "B" }), 400, [1]);

    const removed = grid.removeView([1]);
    expect(removed).toHaveProperty("id", "B");
    expect(grid.root.children).toHaveLength(1);
  });

  it("returns the removed view", () => {
    const grid = new Gridview("horizontal");
    const view = createGridView({ id: "X" });
    grid.addView(view, 400, [0]);

    const removed = grid.removeView([0]);
    expect(removed).toBe(view);
  });

  it("collapses single-child branch after removal", () => {
    const grid = new Gridview("horizontal");
    grid.addView(createGridView({ id: "A" }), 400, [0]);
    grid.addView(createGridView({ id: "B" }), 400, [1]);
    // Restructure: add C below B → root[1] becomes a branch {B, C}
    grid.addView(createGridView({ id: "C" }), 200, [1, 1]);

    expect(grid.root.children[1].kind).toBe("branch");

    // Remove C → branch has only B → should collapse back to a leaf
    grid.removeView([1, 1]);
    expect(grid.root.children[1].kind).toBe("leaf");
    expect((grid.root.children[1] as LeafNode).view).toHaveProperty("id", "B");
  });

  it("classic DCC layout: add 4 panels, remove one, tree stays valid", () => {
    const grid = new Gridview("horizontal");
    // Left panel
    grid.addView(createGridView({ id: "scene" }), 200, [0]);
    // Center viewport
    grid.addView(createGridView({ id: "viewport" }), 500, [1]);
    // Right panel (split viewport → creates branch)
    grid.addViewAt(createGridView({ id: "inspector" }), 200, "right", [1]);
    // Bottom of viewport (split center → creates deeper branch)
    grid.addViewAt(createGridView({ id: "timeline" }), 150, "down", [1]);

    // Should have: scene | (viewport/timeline) | inspector
    expect(grid.root.children).toHaveLength(3);

    // Remove timeline
    grid.removeView([1, 1]);
    // Branch should collapse: just viewport remains at root[1]
    expect(grid.root.children[1].kind).toBe("leaf");
    expect((grid.root.children[1] as LeafNode).view).toHaveProperty("id", "viewport");
  });
});

// ─── Layout ────────────────────────────────────────────────────────────────

describe("Gridview — layout", () => {
  it("layout propagates dimensions to leaf views", () => {
    const grid = new Gridview("horizontal");
    const a = createGridView({ id: "A" });
    const b = createGridView({ id: "B" });
    grid.addView(a, 400, [0]);
    grid.addView(b, 400, [1]);

    grid.layout(800, 600);

    // Both views should have been laid out
    expect(a.lastLayout).toBeDefined();
    expect(b.lastLayout).toBeDefined();
    // Heights should be 600 (the orthogonal dimension)
    expect(a.lastLayout!.h).toBe(600);
    expect(b.lastLayout!.h).toBe(600);
    // Widths should sum to 800
    expect(a.lastLayout!.w + b.lastLayout!.w).toBe(800);
  });

  it("nested layout propagates to all levels", () => {
    const grid = new Gridview("horizontal");
    const a = createGridView({ id: "A" });
    const b = createGridView({ id: "B" });
    const c = createGridView({ id: "C" });
    grid.addView(a, 300, [0]);
    grid.addView(b, 500, [1]);
    grid.addView(c, 250, [1, 1]); // below B

    grid.layout(800, 600);

    expect(a.lastLayout).toBeDefined();
    expect(b.lastLayout).toBeDefined();
    expect(c.lastLayout).toBeDefined();
  });
});

// ─── Serialization ─────────────────────────────────────────────────────────

describe("Gridview — serialization", () => {
  it("serializes a flat layout", () => {
    const grid = new Gridview("horizontal");
    grid.addView(createGridView({ id: "A" }), 400, [0]);
    grid.addView(createGridView({ id: "B" }), 400, [1]);
    grid.layout(800, 600);

    const data = grid.serialize();
    expect(data.orientation).toBe("horizontal");
    expect(data.width).toBe(800);
    expect(data.height).toBe(600);
    expect(data.root.type).toBe("branch");
    expect((data.root as any).data).toHaveLength(2);
  });

  it("serializes a nested layout", () => {
    const grid = new Gridview("horizontal");
    grid.addView(createGridView({ id: "A" }), 300, [0]);
    grid.addView(createGridView({ id: "B" }), 500, [1]);
    grid.addView(createGridView({ id: "C" }), 250, [1, 1]);
    grid.layout(800, 600);

    const data = grid.serialize();
    const rootChildren = (data.root as any).data;
    expect(rootChildren).toHaveLength(2);
    expect(rootChildren[0].type).toBe("leaf");
    expect(rootChildren[1].type).toBe("branch");
    expect(rootChildren[1].data).toHaveLength(2);
  });

  it("deserialize roundtrip preserves structure", () => {
    const grid = new Gridview("horizontal");
    grid.addView(createGridView({ id: "A" }), 300, [0]);
    grid.addView(createGridView({ id: "B" }), 500, [1]);
    grid.addView(createGridView({ id: "C" }), 250, [1, 1]);
    grid.layout(800, 600);

    const data = grid.serialize();
    const restored = Gridview.deserialize(data, (id) => createGridView({ id }));

    expect(restored.orientation).toBe("horizontal");
    expect(restored.root.children).toHaveLength(2);
    expect(restored.root.children[0].kind).toBe("leaf");
    expect(restored.root.children[1].kind).toBe("branch");

    const branch = restored.root.children[1] as BranchNode;
    expect(branch.children).toHaveLength(2);
    expect((branch.children[0] as LeafNode).view).toHaveProperty("id", "B");
    expect((branch.children[1] as LeafNode).view).toHaveProperty("id", "C");
  });

  it("deserialize roundtrip preserves dimensions", () => {
    const grid = new Gridview("horizontal");
    grid.addView(createGridView({ id: "A" }), 300, [0]);
    grid.addView(createGridView({ id: "B" }), 500, [1]);
    grid.layout(800, 600);

    const data = grid.serialize();
    const restored = Gridview.deserialize(data, (id) => createGridView({ id }));

    expect(restored.width).toBe(800);
    expect(restored.height).toBe(600);
  });
});

// ─── Complex Scenarios ─────────────────────────────────────────────────────

describe("Gridview — complex scenarios", () => {
  it("DCC layout: left sidebar, center viewport, right inspector, bottom timeline", () => {
    const grid = new Gridview("horizontal");

    // Start with viewport
    grid.addView(createGridView({ id: "viewport" }), 500, [0]);
    // Add scene to the left
    grid.addViewAt(createGridView({ id: "scene" }), 200, "left", [0]);
    // Add inspector to the right of viewport (now at [1])
    grid.addViewAt(createGridView({ id: "inspector" }), 200, "right", [1]);
    // Add timeline below viewport (now at [1])
    grid.addViewAt(createGridView({ id: "timeline" }), 150, "down", [1]);

    grid.layout(900, 600);

    // Root should have 3 children: scene, branch(viewport+timeline), inspector
    expect(grid.root.children).toHaveLength(3);
    expect(grid.root.children[1].kind).toBe("branch");

    const centerBranch = grid.root.children[1] as BranchNode;
    expect(centerBranch.children).toHaveLength(2);
    expect((centerBranch.children[0] as LeafNode).view).toHaveProperty("id", "viewport");
    expect((centerBranch.children[1] as LeafNode).view).toHaveProperty("id", "timeline");
  });

  it("add then remove all but one view", () => {
    const grid = new Gridview("horizontal");
    grid.addView(createGridView({ id: "A" }), 300, [0]);
    grid.addView(createGridView({ id: "B" }), 300, [1]);
    grid.addView(createGridView({ id: "C" }), 300, [2]);

    grid.removeView([2]);
    grid.removeView([1]);

    expect(grid.root.children).toHaveLength(1);
    expect((grid.root.children[0] as LeafNode).view).toHaveProperty("id", "A");
  });

  it("serialize → deserialize → layout → works", () => {
    const grid = new Gridview("horizontal");
    grid.addView(createGridView({ id: "scene" }), 200, [0]);
    grid.addView(createGridView({ id: "viewport" }), 500, [1]);
    grid.addViewAt(createGridView({ id: "perf" }), 150, "down", [1]);
    grid.layout(700, 600);

    const data = grid.serialize();
    const restored = Gridview.deserialize(data, (id) => createGridView({ id }));
    restored.layout(700, 600);

    // Should not throw and should have the same structure
    expect(restored.root.children).toHaveLength(2);
  });
});
