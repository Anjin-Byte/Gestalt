import { render, fireEvent } from "@testing-library/svelte";
import { describe, it, expect, vi, beforeEach } from "vitest";
import TreeList from "./TreeList.svelte";
import type { TreeListDomain, TreeListItem, TreeListColumnDef } from "./types";

// ContextMenu standalone tests are now in src/ContextMenu.test.ts.
// This file only tests the TreeList ↔ ContextMenu integration.

// ─── TreeList Context Menu Integration ─────────────────────────────────────

describe("TreeList — context menu integration", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  interface TestData {
    items: { id: string; name: string; group: string }[];
  }

  const testData: TestData = {
    items: [
      { id: "r1", name: "Cube", group: "Objects" },
      { id: "r2", name: "Sphere", group: "Objects" },
    ],
  };

  const baseCols: TreeListColumnDef[] = [
    { id: "status", width: 20, label: "Status" },
  ];

  function makeDomain(overrides: Partial<TreeListDomain<TestData>> = {}): TreeListDomain<TestData> {
    return {
      domainId: "ctx-test",
      columns: baseCols,
      rows(data: TestData): TreeListItem[] {
        const items: TreeListItem[] = [];
        items.push({ kind: "group", id: "Objects", label: "Objects", count: data.items.length });
        for (const item of data.items) {
          items.push({
            kind: "row",
            id: item.id,
            groupId: "Objects",
            label: item.name,
            renameable: true,
            cells: [{ type: "status", status: "ok" }],
          });
        }
        return items;
      },
      ...overrides,
    };
  }

  it("right-click opens context menu when domain provides getContextItems", async () => {
    const getContextItems = vi.fn(() => [
      { id: "rename", label: "Rename", shortcut: "F2" },
      { id: "delete", label: "Delete", shortcut: "Del", danger: true },
    ]);
    const { container, getByText } = render(TreeList, {
      domain: makeDomain({ getContextItems, onContextAction: vi.fn() }),
      data: testData,
    });
    await fireEvent.contextMenu(getByText("Cube"));
    expect(getContextItems).toHaveBeenCalledWith("r1");
    expect(container.querySelector(".ctx-menu")).toBeInTheDocument();
    expect(getByText("Rename")).toBeInTheDocument();
  });

  it("right-click does NOT open menu when domain has no getContextItems", async () => {
    const { container, getByText } = render(TreeList, {
      domain: makeDomain(),
      data: testData,
    });
    await fireEvent.contextMenu(getByText("Cube"));
    expect(container.querySelector(".ctx-menu")).not.toBeInTheDocument();
  });

  it("right-click does NOT open menu when getContextItems returns empty array", async () => {
    const getContextItems = vi.fn(() => []);
    const { container, getByText } = render(TreeList, {
      domain: makeDomain({ getContextItems }),
      data: testData,
    });
    await fireEvent.contextMenu(getByText("Cube"));
    expect(container.querySelector(".ctx-menu")).not.toBeInTheDocument();
  });

  it("selecting a context menu item calls onContextAction with rowId and actionId", async () => {
    const onContextAction = vi.fn();
    const getContextItems = () => [
      { id: "rename", label: "Rename" },
    ];
    const { getByText } = render(TreeList, {
      domain: makeDomain({ getContextItems, onContextAction }),
      data: testData,
    });
    // Open menu
    await fireEvent.contextMenu(getByText("Sphere"));
    // Click item
    await fireEvent.click(getByText("Rename"));
    expect(onContextAction).toHaveBeenCalledWith("r2", "rename");
  });

  it("context menu closes after item selection", async () => {
    const getContextItems = () => [{ id: "rename", label: "Rename" }];
    const { container, getByText } = render(TreeList, {
      domain: makeDomain({ getContextItems, onContextAction: vi.fn() }),
      data: testData,
    });
    await fireEvent.contextMenu(getByText("Cube"));
    expect(container.querySelector(".ctx-menu")).toBeInTheDocument();
    await fireEvent.click(getByText("Rename"));
    expect(container.querySelector(".ctx-menu")).not.toBeInTheDocument();
  });

  it("right-click selects the row", async () => {
    const handler = vi.fn();
    const getContextItems = () => [{ id: "a", label: "A" }];
    const { getByText } = render(TreeList, {
      domain: makeDomain({ getContextItems, onContextAction: vi.fn() }),
      data: testData,
      onselectionchange: handler,
    });
    await fireEvent.contextMenu(getByText("Sphere"));
    expect(handler).toHaveBeenCalledWith("r2", "r2");
  });
});
