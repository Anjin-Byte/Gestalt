import { render, fireEvent } from "@testing-library/svelte";
import { describe, it, expect, vi } from "vitest";
import TreeListRow from "./TreeListRow.svelte";
import type { TreeListRowItem, TreeListColumnDef } from "./types";

// ─── Fixtures ──────────────────────────────────────────────────────────────

const baseCols: TreeListColumnDef[] = [
  { id: "status", width: 20, label: "Status" },
  { id: "version", width: 44, label: "Version" },
];

function makeRow(overrides: Partial<TreeListRowItem> = {}): TreeListRowItem {
  return {
    kind: "row",
    id: "row-1",
    groupId: "group-1",
    label: "Test Row",
    cells: [
      { type: "status", status: "ok" },
      { type: "mono", value: "v3" },
    ],
    ...overrides,
  };
}

// ─── Rendering ─────────────────────────────────────────────────────────────

describe("TreeListRow — rendering", () => {
  it("renders the row label", () => {
    const { getByText } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
    });
    expect(getByText("Test Row")).toBeInTheDocument();
  });

  it("renders a status cell with StatusIndicator", () => {
    const { container } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
    });
    expect(container.querySelector(".si-ok")).toBeInTheDocument();
  });

  it("renders a mono cell with value text", () => {
    const { getByText } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
    });
    expect(getByText("v3")).toBeInTheDocument();
  });

  it("renders toggle cells as buttons with aria-pressed", () => {
    const cols: TreeListColumnDef[] = [{ id: "vis", width: 20, label: "Visible" }];
    const row = makeRow({
      cells: [{ type: "toggle", value: true, icon: "👁" }],
    });
    const { container } = render(TreeListRow, { item: row, columns: cols });
    const btn = container.querySelector("button.ol-toggle");
    expect(btn).toBeInTheDocument();
    expect(btn).toHaveAttribute("aria-pressed", "true");
  });

  it("sets data-row-id attribute on the root element", () => {
    const { container } = render(TreeListRow, {
      item: makeRow({ id: "chunk-42" }),
      columns: baseCols,
    });
    expect(container.querySelector("[data-row-id='chunk-42']")).toBeInTheDocument();
  });

  it("sets grid-template-columns based on column widths", () => {
    const { container } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
    });
    const row = container.querySelector(".treelist-row") as HTMLElement;
    expect(row.style.gridTemplateColumns).toBe("1fr 20px 44px");
  });
});

// ─── Visual States ─────────────────────────────────────────────────────────

describe("TreeListRow — visual states", () => {
  it("applies .selected class when selected=true", () => {
    const { container } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
      selected: true,
    });
    expect(container.querySelector(".treelist-row.selected")).toBeInTheDocument();
  });

  it("does not apply .selected class when selected=false", () => {
    const { container } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
      selected: false,
    });
    expect(container.querySelector(".treelist-row.selected")).not.toBeInTheDocument();
  });

  it("applies .active class when active=true", () => {
    const { container } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
      active: true,
    });
    expect(container.querySelector(".treelist-row.active")).toBeInTheDocument();
  });

  it("applies both .selected and .active when both are true", () => {
    const { container } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
      selected: true,
      active: true,
    });
    const row = container.querySelector(".treelist-row");
    expect(row).toHaveClass("selected");
    expect(row).toHaveClass("active");
  });

  it("applies .faded class when item.faded=true", () => {
    const { container } = render(TreeListRow, {
      item: makeRow({ faded: true }),
      columns: baseCols,
    });
    expect(container.querySelector(".treelist-row.faded")).toBeInTheDocument();
  });

  it("does not apply .faded class when item.faded is absent", () => {
    const { container } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
    });
    expect(container.querySelector(".treelist-row.faded")).not.toBeInTheDocument();
  });

  it("applies depth as CSS variable", () => {
    const { container } = render(TreeListRow, {
      item: makeRow({ depth: 3 }),
      columns: baseCols,
    });
    const row = container.querySelector(".treelist-row") as HTMLElement;
    expect(row.style.getPropertyValue("--depth")).toBe("3");
  });

  it("defaults depth to 0 when not specified", () => {
    const { container } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
    });
    const row = container.querySelector(".treelist-row") as HTMLElement;
    expect(row.style.getPropertyValue("--depth")).toBe("0");
  });
});

// ─── Drag Target Zones ────────────────────────────────────────────────────

describe("TreeListRow — drag target zones", () => {
  it("applies .drag-before class when dragTarget='before'", () => {
    const { container } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
      dragTarget: "before",
    });
    expect(container.querySelector(".treelist-row.drag-before")).toBeInTheDocument();
  });

  it("applies .drag-into class when dragTarget='into'", () => {
    const { container } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
      dragTarget: "into",
    });
    expect(container.querySelector(".treelist-row.drag-into")).toBeInTheDocument();
  });

  it("applies .drag-after class when dragTarget='after'", () => {
    const { container } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
      dragTarget: "after",
    });
    expect(container.querySelector(".treelist-row.drag-after")).toBeInTheDocument();
  });

  it("no drag class when dragTarget is null", () => {
    const { container } = render(TreeListRow, {
      item: makeRow(),
      columns: baseCols,
      dragTarget: null,
    });
    const row = container.querySelector(".treelist-row");
    expect(row).not.toHaveClass("drag-before");
    expect(row).not.toHaveClass("drag-into");
    expect(row).not.toHaveClass("drag-after");
  });
});

// ─── Interactions ──────────────────────────────────────────────────────────

describe("TreeListRow — interactions", () => {
  it("calls onclick with row ID and event on click", async () => {
    const handler = vi.fn();
    const { container } = render(TreeListRow, {
      item: makeRow({ id: "click-test" }),
      columns: baseCols,
      onclick: handler,
    });
    await fireEvent.click(container.querySelector(".treelist-row")!);
    expect(handler).toHaveBeenCalledOnce();
    expect(handler.mock.calls[0][0]).toBe("click-test");
    expect(handler.mock.calls[0][1]).toBeInstanceOf(MouseEvent);
  });

  it("toggle cell click does NOT trigger row onclick (stopPropagation)", async () => {
    const rowClick = vi.fn();
    const toggleClick = vi.fn();
    const cols: TreeListColumnDef[] = [{ id: "vis", width: 20, label: "Visible" }];
    const row = makeRow({
      cells: [{ type: "toggle", value: true, icon: "👁" }],
    });
    const { container } = render(TreeListRow, {
      item: row,
      columns: cols,
      onclick: rowClick,
      ontoggle: toggleClick,
    });
    await fireEvent.click(container.querySelector("button.ol-toggle")!);
    expect(toggleClick).toHaveBeenCalledOnce();
    expect(rowClick).not.toHaveBeenCalled();
  });

  it("toggle cell sends correct arguments: rowId, columnId, inverted value, shiftKey", async () => {
    const handler = vi.fn();
    const cols: TreeListColumnDef[] = [{ id: "render", width: 20, label: "Render" }];
    const row = makeRow({
      id: "row-toggle",
      cells: [{ type: "toggle", value: true, icon: "R" }],
    });
    const { container } = render(TreeListRow, {
      item: row,
      columns: cols,
      ontoggle: handler,
    });
    await fireEvent.click(container.querySelector("button.ol-toggle")!);
    expect(handler).toHaveBeenCalledWith("row-toggle", "render", false, false);
  });

  it("disabled toggle cell cannot be clicked", async () => {
    const handler = vi.fn();
    const cols: TreeListColumnDef[] = [{ id: "vis", width: 20, label: "Visible" }];
    const row = makeRow({
      cells: [{ type: "toggle", value: true, icon: "👁", disabled: true }],
    });
    const { container } = render(TreeListRow, {
      item: row,
      columns: cols,
      ontoggle: handler,
    });
    const btn = container.querySelector("button.ol-toggle") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    await fireEvent.click(btn);
    expect(handler).not.toHaveBeenCalled();
  });

  it("toggle button has accessible aria-label from column definition", () => {
    const cols: TreeListColumnDef[] = [{ id: "vis", width: 20, label: "Hide in viewport" }];
    const row = makeRow({
      cells: [{ type: "toggle", value: false, icon: "👁" }],
    });
    const { container } = render(TreeListRow, {
      item: row,
      columns: cols,
    });
    const btn = container.querySelector("button.ol-toggle");
    expect(btn).toHaveAttribute("aria-label", "Hide in viewport");
    expect(btn).toHaveAttribute("aria-pressed", "false");
  });
});

// ─── Edge Cases ────────────────────────────────────────────────────────────

describe("TreeListRow — edge cases", () => {
  it("renders with zero cells", () => {
    const { getByText } = render(TreeListRow, {
      item: makeRow({ cells: [] }),
      columns: [],
    });
    expect(getByText("Test Row")).toBeInTheDocument();
  });

  it("renders with many columns", () => {
    const cols: TreeListColumnDef[] = Array.from({ length: 8 }, (_, i) => ({
      id: `col-${i}`,
      width: 20,
      label: `Col ${i}`,
    }));
    const cells = Array.from({ length: 8 }, (_, i) => ({
      type: "mono" as const,
      value: `v${i}`,
    }));
    const { container } = render(TreeListRow, {
      item: makeRow({ cells }),
      columns: cols,
    });
    const row = container.querySelector(".treelist-row") as HTMLElement;
    // 1fr + 8 × 20px
    expect(row.style.gridTemplateColumns).toBe("1fr 20px 20px 20px 20px 20px 20px 20px 20px");
  });
});
