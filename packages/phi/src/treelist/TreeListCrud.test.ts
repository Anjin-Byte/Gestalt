import { render, fireEvent } from "@testing-library/svelte";
import { describe, it, expect, vi, beforeEach } from "vitest";
import TreeListRow from "./TreeListRow.svelte";
import TreeList from "./TreeList.svelte";
import type { TreeListDomain, TreeListItem, TreeListColumnDef, TreeListRowItem } from "./types";

// ─── Fixtures ──────────────────────────────────────────────────────────────

const baseCols: TreeListColumnDef[] = [
  { id: "status", width: 20, label: "Status" },
];

function makeRow(overrides: Partial<TreeListRowItem> = {}): TreeListRowItem {
  return {
    kind: "row",
    id: "row-1",
    groupId: "group-1",
    label: "Original Name",
    renameable: true,
    cells: [{ type: "status", status: "ok" }],
    ...overrides,
  };
}

interface TestData {
  items: { id: string; name: string; group: string }[];
}

function makeCrudDomain(overrides: Partial<TreeListDomain<TestData>> = {}): TreeListDomain<TestData> {
  return {
    domainId: "crud-test",
    columns: baseCols,
    rows(data: TestData): TreeListItem[] {
      const groups = [...new Set(data.items.map((i) => i.group))];
      const items: TreeListItem[] = [];
      for (const g of groups) {
        const members = data.items.filter((i) => i.group === g);
        items.push({ kind: "group", id: g, label: g, count: members.length });
        for (const item of members) {
          items.push({
            kind: "row",
            id: item.id,
            groupId: g,
            label: item.name,
            renameable: true,
            cells: [{ type: "status", status: "ok" }],
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
    { id: "r1", name: "Cube", group: "Objects" },
    { id: "r2", name: "Sphere", group: "Objects" },
    { id: "r3", name: "Light", group: "Lights" },
  ],
};

// ─── TreeListRow — Inline Rename ───────────────────────────────────────────

describe("TreeListRow — inline rename", () => {
  it("shows input when editing=true", () => {
    const { container } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
      editing: true,
    });
    const input = container.querySelector("input.ol-name-input") as HTMLInputElement;
    expect(input).toBeInTheDocument();
  });

  it("shows label span when editing=false", () => {
    const { container, getByText } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
      editing: false,
    });
    expect(getByText("Original Name")).toBeInTheDocument();
    expect(container.querySelector("input.ol-name-input")).not.toBeInTheDocument();
  });

  it("calls onrename with new value on Enter", async () => {
    const onrename = vi.fn();
    const onrenamecomplete = vi.fn();
    const { container } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
      editing: true,
      onrename,
      onrenamecomplete,
    });
    const input = container.querySelector("input.ol-name-input") as HTMLInputElement;
    // Simulate typing a new name
    await fireEvent.input(input, { target: { value: "Renamed Object" } });
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(onrename).toHaveBeenCalledWith("row-1", "Renamed Object");
    expect(onrenamecomplete).toHaveBeenCalled();
  });

  it("calls onrenamecomplete without onrename on Escape", async () => {
    const onrename = vi.fn();
    const onrenamecomplete = vi.fn();
    const { container } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
      editing: true,
      onrename,
      onrenamecomplete,
    });
    const input = container.querySelector("input.ol-name-input") as HTMLInputElement;
    await fireEvent.keyDown(input, { key: "Escape" });
    expect(onrename).not.toHaveBeenCalled();
    expect(onrenamecomplete).toHaveBeenCalled();
  });

  it("does not call onrename when value unchanged", async () => {
    const onrename = vi.fn();
    const onrenamecomplete = vi.fn();
    const { container } = render(TreeListRow, {
      item: makeRow({ label: "Same Name" }),
      columns: baseCols,
      editing: true,
      onrename,
      onrenamecomplete,
    });
    const input = container.querySelector("input.ol-name-input") as HTMLInputElement;
    // Value will be seeded to "Same Name" by the $effect — simulate no change
    await fireEvent.input(input, { target: { value: "Same Name" } });
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(onrename).not.toHaveBeenCalled();
    expect(onrenamecomplete).toHaveBeenCalled();
  });

  it("does not call onrename when value is empty/whitespace", async () => {
    const onrename = vi.fn();
    const onrenamecomplete = vi.fn();
    const { container } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
      editing: true,
      onrename,
      onrenamecomplete,
    });
    const input = container.querySelector("input.ol-name-input") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "   " } });
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(onrename).not.toHaveBeenCalled();
    expect(onrenamecomplete).toHaveBeenCalled();
  });

  it("rename input click does not trigger row onclick", async () => {
    const rowClick = vi.fn();
    const { container } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
      editing: true,
      onclick: rowClick,
    });
    const input = container.querySelector("input.ol-name-input")!;
    await fireEvent.click(input);
    expect(rowClick).not.toHaveBeenCalled();
  });

  it("has accessible aria-label on rename input", () => {
    const { container } = render(TreeListRow, {
      item: makeRow({ label: "My Object" }),
      columns: baseCols,
      editing: true,
    });
    const input = container.querySelector("input.ol-name-input");
    expect(input).toHaveAttribute("aria-label", "Rename My Object");
  });
});

// ─── TreeList — Double-click rename ────────────────────────────────────────

describe("TreeList — double-click rename", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("enters rename mode on double-click when domain supports onRename", async () => {
    const onRename = vi.fn();
    const { container, getByText } = render(TreeList, {
      domain: makeCrudDomain({ onRename }),
      data: testData,
      selectedId: "r1",
      activeId: "r1",
    });
    // Double-click
    await fireEvent.click(getByText("Cube"), { detail: 2 });
    // Input should appear
    const input = container.querySelector("input.ol-name-input");
    expect(input).toBeInTheDocument();
  });

  it("does NOT enter rename mode when domain has no onRename", async () => {
    const { container, getByText } = render(TreeList, {
      domain: makeCrudDomain(),
      data: testData,
    });
    await fireEvent.click(getByText("Cube"), { detail: 2 });
    expect(container.querySelector("input.ol-name-input")).not.toBeInTheDocument();
  });

  it("does NOT enter rename mode on non-renameable rows", async () => {
    const onRename = vi.fn();
    const domain = makeCrudDomain({ onRename });
    // Override rows to make them not renameable
    const origRows = domain.rows;
    domain.rows = (data) => origRows(data).map(item =>
      item.kind === "row" ? { ...item, renameable: false } : item
    );
    const { container, getByText } = render(TreeList, {
      domain,
      data: testData,
    });
    await fireEvent.click(getByText("Cube"), { detail: 2 });
    expect(container.querySelector("input.ol-name-input")).not.toBeInTheDocument();
  });
});

// ─── TreeList — F2 rename ──────────────────────────────────────────────────

describe("TreeList — F2 rename", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("F2 enters rename mode on the active row", async () => {
    const onRename = vi.fn();
    const { container } = render(TreeList, {
      domain: makeCrudDomain({ onRename }),
      data: testData,
      activeId: "r1",
    });
    await fireEvent.keyDown(container.querySelector(".treelist")!, { key: "F2" });
    expect(container.querySelector("input.ol-name-input")).toBeInTheDocument();
  });

  it("F2 does nothing when no row is active", async () => {
    const onRename = vi.fn();
    const { container } = render(TreeList, {
      domain: makeCrudDomain({ onRename }),
      data: testData,
      activeId: null,
    });
    await fireEvent.keyDown(container.querySelector(".treelist")!, { key: "F2" });
    expect(container.querySelector("input.ol-name-input")).not.toBeInTheDocument();
  });
});

// ─── TreeList — Delete ─────────────────────────────────────────────────────

describe("TreeList — delete", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("Delete key calls domain.onDelete with selected ID", async () => {
    const onDelete = vi.fn();
    const { container } = render(TreeList, {
      domain: makeCrudDomain({ onDelete }),
      data: testData,
      selectedId: "r2",
      activeId: "r2",
    });
    await fireEvent.keyDown(container.querySelector(".ol-scroll")!, { key: "Delete" });
    expect(onDelete).toHaveBeenCalledWith(["r2"]);
  });

  it("Backspace key also calls domain.onDelete", async () => {
    const onDelete = vi.fn();
    const { container } = render(TreeList, {
      domain: makeCrudDomain({ onDelete }),
      data: testData,
      selectedId: "r1",
      activeId: "r1",
    });
    await fireEvent.keyDown(container.querySelector(".ol-scroll")!, { key: "Backspace" });
    expect(onDelete).toHaveBeenCalledWith(["r1"]);
  });

  it("Delete does nothing when no row is selected", async () => {
    const onDelete = vi.fn();
    const { container } = render(TreeList, {
      domain: makeCrudDomain({ onDelete }),
      data: testData,
      selectedId: null,
    });
    await fireEvent.keyDown(container.querySelector(".ol-scroll")!, { key: "Delete" });
    expect(onDelete).not.toHaveBeenCalled();
  });

  it("Delete does nothing when domain has no onDelete", async () => {
    const { container } = render(TreeList, {
      domain: makeCrudDomain(),
      data: testData,
      selectedId: "r1",
      activeId: "r1",
    });
    // Should not throw
    await fireEvent.keyDown(container.querySelector(".ol-scroll")!, { key: "Delete" });
  });
});

// ─── TreeList — Duplicate ──────────────────────────────────────────────────

describe("TreeList — duplicate", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("Ctrl+D calls domain.onDuplicate with selected ID", async () => {
    const onDuplicate = vi.fn();
    const { container } = render(TreeList, {
      domain: makeCrudDomain({ onDuplicate }),
      data: testData,
      selectedId: "r1",
      activeId: "r1",
    });
    await fireEvent.keyDown(container.querySelector(".ol-scroll")!, { key: "d", ctrlKey: true });
    expect(onDuplicate).toHaveBeenCalledWith(["r1"]);
  });

  it("Cmd+D also calls domain.onDuplicate (Mac)", async () => {
    const onDuplicate = vi.fn();
    const { container } = render(TreeList, {
      domain: makeCrudDomain({ onDuplicate }),
      data: testData,
      selectedId: "r3",
      activeId: "r3",
    });
    await fireEvent.keyDown(container.querySelector(".ol-scroll")!, { key: "d", metaKey: true });
    expect(onDuplicate).toHaveBeenCalledWith(["r3"]);
  });

  it("plain D does NOT trigger duplicate", async () => {
    const onDuplicate = vi.fn();
    const { container } = render(TreeList, {
      domain: makeCrudDomain({ onDuplicate }),
      data: testData,
      selectedId: "r1",
      activeId: "r1",
    });
    await fireEvent.keyDown(container.querySelector(".ol-scroll")!, { key: "d" });
    expect(onDuplicate).not.toHaveBeenCalled();
  });

  it("Ctrl+D does nothing when no row is selected", async () => {
    const onDuplicate = vi.fn();
    const { container } = render(TreeList, {
      domain: makeCrudDomain({ onDuplicate }),
      data: testData,
      selectedId: null,
    });
    await fireEvent.keyDown(container.querySelector(".ol-scroll")!, { key: "d", ctrlKey: true });
    expect(onDuplicate).not.toHaveBeenCalled();
  });
});
