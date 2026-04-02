import { render, fireEvent } from "@testing-library/svelte";
import { describe, it, expect, vi } from "vitest";
import ContextMenu from "./ContextMenu.svelte";
import type { ContextMenuItem } from "./ContextMenu.svelte";

// ─── Rendering ─────────────────────────────────────────────────────────────

describe("ContextMenu — rendering", () => {
  const baseItems: ContextMenuItem[] = [
    { id: "rename", label: "Rename", shortcut: "F2" },
    { id: "duplicate", label: "Duplicate", shortcut: "⌘D" },
    { id: "delete", label: "Delete", shortcut: "Del", danger: true, separator: true },
  ];

  it("renders all menu items", () => {
    const { getByText } = render(ContextMenu, {
      items: baseItems, x: 100, y: 100, onaction: vi.fn(), onclose: vi.fn(),
    });
    expect(getByText("Rename")).toBeInTheDocument();
    expect(getByText("Duplicate")).toBeInTheDocument();
    expect(getByText("Delete")).toBeInTheDocument();
  });

  it("renders shortcut hints", () => {
    const { getByText } = render(ContextMenu, {
      items: baseItems, x: 100, y: 100, onaction: vi.fn(), onclose: vi.fn(),
    });
    expect(getByText("F2")).toBeInTheDocument();
    expect(getByText("⌘D")).toBeInTheDocument();
    expect(getByText("Del")).toBeInTheDocument();
  });

  it("renders separator before items with separator=true", () => {
    const { container } = render(ContextMenu, {
      items: baseItems, x: 100, y: 100, onaction: vi.fn(), onclose: vi.fn(),
    });
    expect(container.querySelector(".ctx-separator")).toBeInTheDocument();
  });

  it("applies danger class to danger items", () => {
    const { container } = render(ContextMenu, {
      items: baseItems, x: 100, y: 100, onaction: vi.fn(), onclose: vi.fn(),
    });
    expect(container.querySelector(".ctx-danger")).toBeInTheDocument();
  });

  it("applies disabled class to disabled items", () => {
    const items: ContextMenuItem[] = [
      { id: "noop", label: "Cannot Do This", disabled: true },
    ];
    const { container } = render(ContextMenu, {
      items, x: 100, y: 100, onaction: vi.fn(), onclose: vi.fn(),
    });
    expect(container.querySelector(".ctx-disabled")).toBeInTheDocument();
  });

  it("has role=menu on the menu element", () => {
    const { container } = render(ContextMenu, {
      items: baseItems, x: 100, y: 100, onaction: vi.fn(), onclose: vi.fn(),
    });
    expect(container.querySelector("[role='menu']")).toBeInTheDocument();
  });
});

// ─── Interactions ──────────────────────────────────────────────────────────

describe("ContextMenu — interactions", () => {
  const items: ContextMenuItem[] = [
    { id: "rename", label: "Rename" },
    { id: "dup", label: "Duplicate" },
    { id: "disabled-item", label: "Disabled", disabled: true },
    { id: "delete", label: "Delete", danger: true },
  ];

  it("calls onaction and onclose when a menu item is clicked", async () => {
    const onaction = vi.fn();
    const onclose = vi.fn();
    const { getByText } = render(ContextMenu, {
      items, x: 100, y: 100, onaction, onclose,
    });
    await fireEvent.click(getByText("Duplicate"));
    expect(onaction).toHaveBeenCalledWith("dup");
    expect(onclose).toHaveBeenCalled();
  });

  it("does NOT call onaction when a disabled item is clicked", async () => {
    const onaction = vi.fn();
    const onclose = vi.fn();
    const { getByText } = render(ContextMenu, {
      items, x: 100, y: 100, onaction, onclose,
    });
    await fireEvent.click(getByText("Disabled"));
    expect(onaction).not.toHaveBeenCalled();
    expect(onclose).not.toHaveBeenCalled();
  });

  it("calls onclose when backdrop is clicked", async () => {
    const onclose = vi.fn();
    const { container } = render(ContextMenu, {
      items, x: 100, y: 100, onaction: vi.fn(), onclose,
    });
    await fireEvent.click(container.querySelector(".ctx-backdrop")!);
    expect(onclose).toHaveBeenCalled();
  });

  it("calls onclose on Escape key", async () => {
    const onclose = vi.fn();
    const { container } = render(ContextMenu, {
      items, x: 100, y: 100, onaction: vi.fn(), onclose,
    });
    await fireEvent.keyDown(container.querySelector(".ctx-menu")!, { key: "Escape" });
    expect(onclose).toHaveBeenCalled();
  });

  it("ArrowDown navigates to next enabled item", async () => {
    const { container } = render(ContextMenu, {
      items, x: 100, y: 100, onaction: vi.fn(), onclose: vi.fn(),
    });
    const menu = container.querySelector(".ctx-menu")!;
    await fireEvent.keyDown(menu, { key: "ArrowDown" });
    const focused = container.querySelectorAll(".ctx-focused");
    expect(focused.length).toBe(1);
    expect(focused[0].textContent).toContain("Rename");
  });

  it("ArrowDown skips disabled items", async () => {
    const { container } = render(ContextMenu, {
      items, x: 100, y: 100, onaction: vi.fn(), onclose: vi.fn(),
    });
    const menu = container.querySelector(".ctx-menu")!;
    await fireEvent.keyDown(menu, { key: "ArrowDown" }); // rename
    await fireEvent.keyDown(menu, { key: "ArrowDown" }); // dup
    await fireEvent.keyDown(menu, { key: "ArrowDown" }); // delete (skips disabled)
    const focused = container.querySelector(".ctx-focused");
    expect(focused?.textContent).toContain("Delete");
  });

  it("ArrowDown wraps around from last to first", async () => {
    const { container } = render(ContextMenu, {
      items, x: 100, y: 100, onaction: vi.fn(), onclose: vi.fn(),
    });
    const menu = container.querySelector(".ctx-menu")!;
    await fireEvent.keyDown(menu, { key: "ArrowDown" }); // rename
    await fireEvent.keyDown(menu, { key: "ArrowDown" }); // dup
    await fireEvent.keyDown(menu, { key: "ArrowDown" }); // delete
    await fireEvent.keyDown(menu, { key: "ArrowDown" }); // wrap → rename
    const focused = container.querySelector(".ctx-focused");
    expect(focused?.textContent).toContain("Rename");
  });

  it("ArrowUp wraps around from first to last", async () => {
    const { container } = render(ContextMenu, {
      items, x: 100, y: 100, onaction: vi.fn(), onclose: vi.fn(),
    });
    const menu = container.querySelector(".ctx-menu")!;
    await fireEvent.keyDown(menu, { key: "ArrowUp" }); // wrap → delete (last enabled)
    const focused = container.querySelector(".ctx-focused");
    expect(focused?.textContent).toContain("Delete");
  });

  it("Enter selects the focused item", async () => {
    const onaction = vi.fn();
    const onclose = vi.fn();
    const { container } = render(ContextMenu, {
      items, x: 100, y: 100, onaction, onclose,
    });
    const menu = container.querySelector(".ctx-menu")!;
    await fireEvent.keyDown(menu, { key: "ArrowDown" });
    await fireEvent.keyDown(menu, { key: "Enter" });
    expect(onaction).toHaveBeenCalledWith("rename");
    expect(onclose).toHaveBeenCalled();
  });

  it("Enter does nothing when no item is focused", async () => {
    const onaction = vi.fn();
    const { container } = render(ContextMenu, {
      items, x: 100, y: 100, onaction, onclose: vi.fn(),
    });
    await fireEvent.keyDown(container.querySelector(".ctx-menu")!, { key: "Enter" });
    expect(onaction).not.toHaveBeenCalled();
  });

  it("hover highlights item", async () => {
    const { getByText } = render(ContextMenu, {
      items, x: 100, y: 100, onaction: vi.fn(), onclose: vi.fn(),
    });
    await fireEvent.pointerEnter(getByText("Duplicate").closest(".ctx-item")!);
    expect(getByText("Duplicate").closest(".ctx-item")).toHaveClass("ctx-focused");
  });

  it("hover does not highlight disabled item", async () => {
    const { getByText } = render(ContextMenu, {
      items, x: 100, y: 100, onaction: vi.fn(), onclose: vi.fn(),
    });
    await fireEvent.pointerEnter(getByText("Disabled").closest(".ctx-item")!);
    expect(getByText("Disabled").closest(".ctx-item")).not.toHaveClass("ctx-focused");
  });
});
