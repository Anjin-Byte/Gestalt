import { render, fireEvent } from "@testing-library/svelte";
import { describe, it, expect, beforeEach } from "vitest";
import TreeListGroup from "./TreeListGroup.svelte";
import { TreeListStateStore } from "./types";
import type { TreeListGroupItem, TreeListColumnDef } from "./types";

// ─── Fixtures ──────────────────────────────────────────────────────────────

const baseCols: TreeListColumnDef[] = [
  { id: "status", width: 20, label: "Status" },
  { id: "version", width: 44, label: "Version" },
];

function makeGroup(overrides: Partial<TreeListGroupItem> = {}): TreeListGroupItem {
  return {
    kind: "group",
    id: "grp-1",
    label: "Chunks",
    ...overrides,
  };
}

// ─── Tests ─────────────────────────────────────────────────────────────────

describe("TreeListGroup — rendering", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("renders the group label in uppercase", () => {
    const store = new TreeListStateStore("test");
    const { getByText } = render(TreeListGroup, {
      item: makeGroup(),
      stateStore: store,
      columns: baseCols,
    });
    expect(getByText("Chunks")).toBeInTheDocument();
  });

  it("renders the count badge when count is provided", () => {
    const store = new TreeListStateStore("test");
    const { getByText } = render(TreeListGroup, {
      item: makeGroup({ count: 42 }),
      stateStore: store,
      columns: baseCols,
    });
    expect(getByText("42")).toBeInTheDocument();
  });

  it("does not render count badge when count is absent", () => {
    const store = new TreeListStateStore("test");
    const { container } = render(TreeListGroup, {
      item: makeGroup(),
      stateStore: store,
      columns: baseCols,
    });
    expect(container.querySelector(".og-count")).not.toBeInTheDocument();
  });

  it("renders count badge when count is 0", () => {
    const store = new TreeListStateStore("test");
    const { getByText } = render(TreeListGroup, {
      item: makeGroup({ count: 0 }),
      stateStore: store,
      columns: baseCols,
    });
    expect(getByText("0")).toBeInTheDocument();
  });

  it("sets data-group-id attribute", () => {
    const store = new TreeListStateStore("test");
    const { container } = render(TreeListGroup, {
      item: makeGroup({ id: "my-group" }),
      stateStore: store,
      columns: baseCols,
    });
    expect(container.querySelector("[data-group-id='my-group']")).toBeInTheDocument();
  });

  it("sets grid-template-columns matching column widths", () => {
    const store = new TreeListStateStore("test");
    const { container } = render(TreeListGroup, {
      item: makeGroup(),
      stateStore: store,
      columns: baseCols,
    });
    const el = container.querySelector(".treelist-group") as HTMLElement;
    expect(el.style.gridTemplateColumns).toBe("1fr 20px 44px");
  });
});

describe("TreeListGroup — collapse behavior", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("starts expanded (aria-expanded=true) by default", () => {
    const store = new TreeListStateStore("test");
    const { container } = render(TreeListGroup, {
      item: makeGroup(),
      stateStore: store,
      columns: baseCols,
    });
    expect(container.querySelector("[aria-expanded='true']")).toBeInTheDocument();
  });

  it("starts collapsed when stateStore has the group collapsed", () => {
    const store = new TreeListStateStore("test");
    store.collapse("grp-1");
    const { container } = render(TreeListGroup, {
      item: makeGroup(),
      stateStore: store,
      columns: baseCols,
    });
    expect(container.querySelector("[aria-expanded='false']")).toBeInTheDocument();
  });

  it("toggles collapsed state on click", async () => {
    const store = new TreeListStateStore("test");
    const { container } = render(TreeListGroup, {
      item: makeGroup(),
      stateStore: store,
      columns: baseCols,
    });
    const btn = container.querySelector(".treelist-group")!;

    // Initially expanded
    expect(btn).toHaveAttribute("aria-expanded", "true");
    expect(store.isCollapsed("grp-1")).toBe(false);

    // Click → collapsed
    await fireEvent.click(btn);
    expect(store.isCollapsed("grp-1")).toBe(true);
  });

  it("chevron has .open class when expanded", () => {
    const store = new TreeListStateStore("test");
    const { container } = render(TreeListGroup, {
      item: makeGroup(),
      stateStore: store,
      columns: baseCols,
    });
    expect(container.querySelector(".og-chevron.open")).toBeInTheDocument();
  });

  it("chevron does not have .open class when collapsed", () => {
    const store = new TreeListStateStore("test");
    store.collapse("grp-1");
    const { container } = render(TreeListGroup, {
      item: makeGroup(),
      stateStore: store,
      columns: baseCols,
    });
    expect(container.querySelector(".og-chevron.open")).not.toBeInTheDocument();
  });

  it("persists toggle to localStorage via TreeListStateStore", async () => {
    const store = new TreeListStateStore("persist-grp");
    const { container } = render(TreeListGroup, {
      item: makeGroup(),
      stateStore: store,
      columns: baseCols,
    });
    await fireEvent.click(container.querySelector(".treelist-group")!);
    const raw = localStorage.getItem("treelist:persist-grp:collapsed");
    expect(raw).not.toBeNull();
    expect(JSON.parse(raw!)).toContain("grp-1");
  });
});

describe("TreeListGroup — aggregate", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("renders BarMeter when aggregate is provided", () => {
    const store = new TreeListStateStore("test");
    const { container } = render(TreeListGroup, {
      item: makeGroup({ aggregate: { value: 50, max: 100 } }),
      stateStore: store,
      columns: baseCols,
    });
    // BarMeter renders a .bar-meter element
    expect(container.querySelector(".bar-meter")).toBeInTheDocument();
  });

  it("does not render BarMeter when aggregate is absent", () => {
    const store = new TreeListStateStore("test");
    const { container } = render(TreeListGroup, {
      item: makeGroup(),
      stateStore: store,
      columns: baseCols,
    });
    expect(container.querySelector(".bar-meter")).not.toBeInTheDocument();
  });
});
