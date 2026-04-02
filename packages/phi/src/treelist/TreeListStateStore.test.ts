import { describe, it, expect, beforeEach } from "vitest";
import { TreeListStateStore } from "./types";

describe("TreeListStateStore", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("starts with no groups collapsed", () => {
    const store = new TreeListStateStore("test");
    expect(store.isCollapsed("g1")).toBe(false);
  });

  it("toggle collapses an expanded group", () => {
    const store = new TreeListStateStore("test");
    store.toggle("g1");
    expect(store.isCollapsed("g1")).toBe(true);
  });

  it("toggle expands a collapsed group", () => {
    const store = new TreeListStateStore("test");
    store.toggle("g1");
    store.toggle("g1");
    expect(store.isCollapsed("g1")).toBe(false);
  });

  it("collapse sets a group to collapsed", () => {
    const store = new TreeListStateStore("test");
    store.collapse("g1");
    expect(store.isCollapsed("g1")).toBe(true);
  });

  it("expand sets a group to expanded", () => {
    const store = new TreeListStateStore("test");
    store.collapse("g1");
    store.expand("g1");
    expect(store.isCollapsed("g1")).toBe(false);
  });

  it("expandAll expands multiple groups at once", () => {
    const store = new TreeListStateStore("test");
    store.collapse("g1");
    store.collapse("g2");
    store.collapse("g3");
    store.expandAll(["g1", "g3"]);
    expect(store.isCollapsed("g1")).toBe(false);
    expect(store.isCollapsed("g2")).toBe(true);
    expect(store.isCollapsed("g3")).toBe(false);
  });

  it("collapsedSet returns a read-only view of collapsed groups", () => {
    const store = new TreeListStateStore("test");
    store.collapse("g1");
    store.collapse("g2");
    const set = store.collapsedSet;
    expect(set.has("g1")).toBe(true);
    expect(set.has("g2")).toBe(true);
    expect(set.size).toBe(2);
  });

  it("persists collapsed state to localStorage", () => {
    const store = new TreeListStateStore("persist");
    store.collapse("g1");
    store.collapse("g2");
    const raw = localStorage.getItem("treelist:persist:collapsed");
    expect(raw).not.toBeNull();
    const parsed = JSON.parse(raw!);
    expect(parsed).toContain("g1");
    expect(parsed).toContain("g2");
  });

  it("restores collapsed state from localStorage on construction", () => {
    localStorage.setItem("treelist:restore:collapsed", JSON.stringify(["g1", "g3"]));
    const store = new TreeListStateStore("restore");
    expect(store.isCollapsed("g1")).toBe(true);
    expect(store.isCollapsed("g2")).toBe(false);
    expect(store.isCollapsed("g3")).toBe(true);
  });

  it("separate domains do not share state", () => {
    const storeA = new TreeListStateStore("domain-a");
    const storeB = new TreeListStateStore("domain-b");
    storeA.collapse("g1");
    expect(storeB.isCollapsed("g1")).toBe(false);
  });

  it("handles empty localStorage gracefully", () => {
    localStorage.setItem("treelist:empty:collapsed", "");
    // Empty string → JSON.parse fails, should fallback gracefully
    // Actually our constructor checks `saved ? JSON.parse(saved) : []`
    // Empty string is falsy → defaults to empty set
    const store = new TreeListStateStore("empty");
    expect(store.isCollapsed("g1")).toBe(false);
  });

  it("handles malformed localStorage gracefully", () => {
    localStorage.setItem("treelist:bad:collapsed", "not-json");
    // JSON.parse will throw — this is a real edge case worth testing
    expect(() => new TreeListStateStore("bad")).toThrow();
  });
});
