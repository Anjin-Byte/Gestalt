import { render, fireEvent } from "@testing-library/svelte";
import { describe, it, expect, vi, beforeEach } from "vitest";
import TreeList from "./TreeList.svelte";
import type { TreeListDomain, TreeListItem, TreeListColumnDef } from "./types";

// ─── Test Domain ───────────────────────────────────────────────────────────

interface TestData {
  items: { id: string; name: string; group: string; status: "ok" | "warning" | "error" | "idle" }[];
}

const testColumns: TreeListColumnDef[] = [
  { id: "status", width: 20, label: "Status" },
  { id: "version", width: 44, label: "Version" },
];

function makeDomain(overrides: Partial<TreeListDomain<TestData>> = {}): TreeListDomain<TestData> {
  return {
    domainId: "test",
    columns: testColumns,
    rows(data: TestData): TreeListItem[] {
      const groups = new Set(data.items.map((i) => i.group));
      const items: TreeListItem[] = [];
      for (const g of groups) {
        const groupItems = data.items.filter((i) => i.group === g);
        items.push({ kind: "group", id: g, label: g, count: groupItems.length });
        for (const item of groupItems) {
          items.push({
            kind: "row",
            id: item.id,
            groupId: g,
            label: item.name,
            cells: [
              { type: "status", status: item.status },
              { type: "mono", value: "v1" },
            ],
          });
        }
      }
      return items;
    },
    ...overrides,
  };
}

const testData: TestData = {
  items: [
    { id: "r1", name: "Chunk 0,0,0", group: "Active", status: "ok" },
    { id: "r2", name: "Chunk 1,0,0", group: "Active", status: "ok" },
    { id: "r3", name: "Chunk 0,1,0", group: "Active", status: "warning" },
    { id: "r4", name: "Buffer A", group: "Pooled", status: "idle" },
    { id: "r5", name: "Buffer B", group: "Pooled", status: "error" },
  ],
};

// ─── Rendering ─────────────────────────────────────────────────────────────

describe("TreeList — rendering", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("renders all groups and rows", () => {
    const { getByText } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
    });
    expect(getByText("Active")).toBeInTheDocument();
    expect(getByText("Pooled")).toBeInTheDocument();
    expect(getByText("Chunk 0,0,0")).toBeInTheDocument();
    expect(getByText("Buffer A")).toBeInTheDocument();
    expect(getByText("Buffer B")).toBeInTheDocument();
  });

  it("sets data-domain attribute on root", () => {
    const { container } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
    });
    expect(container.querySelector("[data-domain='test']")).toBeInTheDocument();
  });

  it("renders filter input", () => {
    const { container } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
    });
    expect(container.querySelector("input[type='search']")).toBeInTheDocument();
  });

  it("renders empty when data produces no items", () => {
    const { container } = render(TreeList, {
      domain: makeDomain(),
      data: { items: [] },
    });
    expect(container.querySelector(".treelist-row")).not.toBeInTheDocument();
    expect(container.querySelector(".treelist-group")).not.toBeInTheDocument();
  });
});

// ─── Collapse ──────────────────────────────────────────────────────────────

describe("TreeList — collapse", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("hides rows when a group is clicked (collapsed)", async () => {
    const { container, queryByText } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
    });
    const activeGroup = container.querySelector("[data-group-id='Active']")!;
    await fireEvent.click(activeGroup);

    // Rows under "Active" should be hidden
    expect(queryByText("Chunk 0,0,0")).not.toBeInTheDocument();
    expect(queryByText("Chunk 1,0,0")).not.toBeInTheDocument();

    // Rows under "Pooled" should still be visible
    expect(queryByText("Buffer A")).toBeInTheDocument();
  });

  it("re-shows rows when a collapsed group is clicked again", async () => {
    const { container, getByText, queryByText } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
    });
    const activeGroup = container.querySelector("[data-group-id='Active']")!;

    // Collapse
    await fireEvent.click(activeGroup);
    expect(queryByText("Chunk 0,0,0")).not.toBeInTheDocument();

    // Expand
    await fireEvent.click(activeGroup);
    expect(getByText("Chunk 0,0,0")).toBeInTheDocument();
  });

  it("persists collapse state to localStorage", async () => {
    const { container } = render(TreeList, {
      domain: makeDomain({ domainId: "persist-test" }),
      data: testData,
    });
    await fireEvent.click(container.querySelector("[data-group-id='Active']")!);
    const raw = localStorage.getItem("treelist:persist-test:collapsed");
    expect(raw).not.toBeNull();
    expect(JSON.parse(raw!)).toContain("Active");
  });

  it("restores collapse state from localStorage", () => {
    localStorage.setItem("treelist:restore-test:collapsed", JSON.stringify(["Pooled"]));
    const { queryByText, getByText } = render(TreeList, {
      domain: makeDomain({ domainId: "restore-test" }),
      data: testData,
    });
    // Pooled rows should be hidden
    expect(queryByText("Buffer A")).not.toBeInTheDocument();
    // Active rows should be visible
    expect(getByText("Chunk 0,0,0")).toBeInTheDocument();
  });
});

// ─── Filter ────────────────────────────────────────────────────────────────

describe("TreeList — filter", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("filters rows by label substring (case-insensitive)", async () => {
    const { container, queryByText, getByText } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
    });
    const input = container.querySelector("input[type='search']") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "chunk" } });

    // Chunk rows visible
    expect(getByText("Chunk 0,0,0")).toBeInTheDocument();
    expect(getByText("Chunk 1,0,0")).toBeInTheDocument();

    // Buffer rows hidden
    expect(queryByText("Buffer A")).not.toBeInTheDocument();
    expect(queryByText("Buffer B")).not.toBeInTheDocument();
  });

  it("hides groups with no matching rows", async () => {
    const { container, queryByText, getByText } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
    });
    const input = container.querySelector("input[type='search']") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "Buffer" } });

    // Pooled group visible
    expect(getByText("Pooled")).toBeInTheDocument();
    // Active group hidden — no matching rows
    expect(queryByText("Active")).not.toBeInTheDocument();
  });

  it("shows all rows when filter is cleared", async () => {
    const { container, getByText } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
    });
    const input = container.querySelector("input[type='search']") as HTMLInputElement;

    // Filter
    await fireEvent.input(input, { target: { value: "Buffer" } });
    // Clear
    await fireEvent.input(input, { target: { value: "" } });

    expect(getByText("Chunk 0,0,0")).toBeInTheDocument();
    expect(getByText("Buffer A")).toBeInTheDocument();
  });

  it("filter ignores collapse state — shows matching rows even in collapsed groups", async () => {
    const { container, getByText } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
    });

    // Collapse the Active group
    await fireEvent.click(container.querySelector("[data-group-id='Active']")!);

    // Filter for a chunk — should still appear even though group is collapsed
    const input = container.querySelector("input[type='search']") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "Chunk 0,0" } });

    expect(getByText("Chunk 0,0,0")).toBeInTheDocument();
  });

  it("no results when filter matches nothing", async () => {
    const { container, queryByText } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
    });
    const input = container.querySelector("input[type='search']") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "zzz_no_match" } });

    expect(queryByText("Active")).not.toBeInTheDocument();
    expect(queryByText("Pooled")).not.toBeInTheDocument();
    expect(queryByText("Chunk 0,0,0")).not.toBeInTheDocument();
  });
});

// ─── Selection ─────────────────────────────────────────────────────────────

describe("TreeList — selection", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("calls onselectionchange when a row is clicked", async () => {
    const handler = vi.fn();
    const { getByText } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
      onselectionchange: handler,
    });
    await fireEvent.click(getByText("Chunk 0,0,0"));
    expect(handler).toHaveBeenCalledWith("r1", "r1");
  });

  it("calls domain.onSelect when a row is clicked", async () => {
    const onSelect = vi.fn();
    const { getByText } = render(TreeList, {
      domain: makeDomain({ onSelect }),
      data: testData,
    });
    await fireEvent.click(getByText("Buffer A"));
    expect(onSelect).toHaveBeenCalledWith("r4");
  });

  it("applies .selected class to the selected row", () => {
    const { container } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
      selectedId: "r1",
    });
    const row = container.querySelector("[data-row-id='r1']");
    expect(row).toHaveClass("selected");
  });

  it("applies .active class to the active row", () => {
    const { container } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
      activeId: "r2",
    });
    const row = container.querySelector("[data-row-id='r2']");
    expect(row).toHaveClass("active");
  });

  it("only one row has .selected at a time", () => {
    const { container } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
      selectedId: "r3",
    });
    const selectedRows = container.querySelectorAll(".treelist-row.selected");
    expect(selectedRows.length).toBe(1);
  });
});

// ─── Keyboard Navigation ───────────────────────────────────────────────────

describe("TreeList — keyboard navigation", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("ArrowDown moves selection to first row when nothing is selected", async () => {
    const handler = vi.fn();
    const { container } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
      onselectionchange: handler,
    });
    const scroll = container.querySelector(".ol-scroll")!;
    await fireEvent.keyDown(scroll, { key: "ArrowDown" });
    // First navigable row should be selected
    expect(handler).toHaveBeenCalledWith("r1", "r1");
  });

  it("ArrowDown advances to the next row", async () => {
    const handler = vi.fn();
    const { container } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
      activeId: "r1",
      onselectionchange: handler,
    });
    const scroll = container.querySelector(".ol-scroll")!;
    await fireEvent.keyDown(scroll, { key: "ArrowDown" });
    expect(handler).toHaveBeenCalledWith("r2", "r2");
  });

  it("ArrowUp moves to the previous row", async () => {
    const handler = vi.fn();
    const { container } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
      activeId: "r2",
      onselectionchange: handler,
    });
    const scroll = container.querySelector(".ol-scroll")!;
    await fireEvent.keyDown(scroll, { key: "ArrowUp" });
    expect(handler).toHaveBeenCalledWith("r1", "r1");
  });

  it("ArrowDown does not go past the last row", async () => {
    const handler = vi.fn();
    const { container } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
      activeId: "r5",
      onselectionchange: handler,
    });
    const scroll = container.querySelector(".ol-scroll")!;
    await fireEvent.keyDown(scroll, { key: "ArrowDown" });
    expect(handler).toHaveBeenCalledWith("r5", "r5");
  });

  it("ArrowUp does not go before the first row", async () => {
    const handler = vi.fn();
    const { container } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
      activeId: "r1",
      onselectionchange: handler,
    });
    const scroll = container.querySelector(".ol-scroll")!;
    await fireEvent.keyDown(scroll, { key: "ArrowUp" });
    expect(handler).toHaveBeenCalledWith("r1", "r1");
  });

  it("Escape clears filter when filter is active", async () => {
    const { container } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
    });
    const input = container.querySelector("input[type='search']") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "chunk" } });

    // Escape on the treelist container
    const treelist = container.querySelector(".treelist")!;
    await fireEvent.keyDown(treelist, { key: "Escape" });

    // Filter should be cleared — all items visible again
    expect(container.querySelector("[data-row-id='r4']")).toBeInTheDocument();
  });

  it("Escape clears selection when no filter is active", async () => {
    const handler = vi.fn();
    const { container } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
      selectedId: "r1",
      activeId: "r1",
      onselectionchange: handler,
    });
    const treelist = container.querySelector(".treelist")!;
    await fireEvent.keyDown(treelist, { key: "Escape" });
    expect(handler).toHaveBeenCalledWith(null, null);
  });
});

// ─── Toggle Passthrough ────────────────────────────────────────────────────

describe("TreeList — toggle passthrough", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("calls domain.onToggle when a toggle cell is clicked", async () => {
    const onToggle = vi.fn();
    const cols: TreeListColumnDef[] = [{ id: "vis", width: 20, label: "Visible" }];
    const domain: TreeListDomain<TestData> = {
      domainId: "toggle-test",
      columns: cols,
      rows: () => [
        { kind: "group", id: "g", label: "G" },
        {
          kind: "row",
          id: "row-t",
          groupId: "g",
          label: "Test",
          cells: [{ type: "toggle", value: true, icon: "👁" }],
        },
      ],
      onToggle,
    };
    const { container } = render(TreeList, { domain, data: testData });
    await fireEvent.click(container.querySelector("button.ol-toggle")!);
    expect(onToggle).toHaveBeenCalledWith("row-t", "vis", false, false);
  });

  it("toggle click does NOT trigger row selection", async () => {
    const onSelect = vi.fn();
    const cols: TreeListColumnDef[] = [{ id: "vis", width: 20, label: "Visible" }];
    const domain: TreeListDomain<TestData> = {
      domainId: "toggle-nosel",
      columns: cols,
      rows: () => [
        { kind: "group", id: "g", label: "G" },
        {
          kind: "row",
          id: "row-t",
          groupId: "g",
          label: "Test",
          cells: [{ type: "toggle", value: true, icon: "👁" }],
        },
      ],
      onSelect,
    };
    const { container } = render(TreeList, { domain, data: testData });
    await fireEvent.click(container.querySelector("button.ol-toggle")!);
    expect(onSelect).not.toHaveBeenCalled();
  });
});

// ─── Overflow ──────────────────────────────────────────────────────────────

describe("TreeList — overflow", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("shows overflow indicator when items exceed maxRows", () => {
    const manyItems: TestData = {
      items: Array.from({ length: 10 }, (_, i) => ({
        id: `r${i}`,
        name: `Row ${i}`,
        group: "All",
        status: "ok" as const,
      })),
    };
    const { getByText } = render(TreeList, {
      domain: makeDomain(),
      data: manyItems,
      maxRows: 5,
    });
    // 1 group + 10 rows = 11 items, maxRows=5 → 6 hidden
    expect(getByText("(6 more hidden)")).toBeInTheDocument();
  });

  it("does not show overflow indicator when items fit", () => {
    const { container } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
      maxRows: 200,
    });
    expect(container.querySelector(".ol-overflow")).not.toBeInTheDocument();
  });
});
