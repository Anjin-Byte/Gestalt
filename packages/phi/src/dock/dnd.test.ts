import { describe, it, expect } from "vitest";
import { detectZone, zoneToDirection } from "./dnd";

// Helper to create a DOMRect-like object
function rect(x: number, y: number, w: number, h: number): DOMRect {
  return { left: x, top: y, width: w, height: h, right: x + w, bottom: y + h, x, y, toJSON: () => ({}) };
}

describe("detectZone", () => {
  const r = rect(100, 200, 500, 400);

  it("returns center for the middle of the rect", () => {
    expect(detectZone(r, 350, 400)).toBe("center");
  });

  it("returns center for 50% point", () => {
    expect(detectZone(r, 350, 400)).toBe("center");
  });

  it("returns left for 10% from left edge", () => {
    // 10% of 500 = 50px → x = 100 + 50 = 150
    expect(detectZone(r, 140, 400)).toBe("left");
  });

  it("returns right for 10% from right edge", () => {
    // 90% of 500 = 450px → x = 100 + 450 = 550
    expect(detectZone(r, 560, 400)).toBe("right");
  });

  it("returns top for 10% from top edge", () => {
    // 10% of 400 = 40px → y = 200 + 40 = 240
    expect(detectZone(r, 350, 230)).toBe("top");
  });

  it("returns bottom for 10% from bottom edge", () => {
    // 90% of 400 = 360px → y = 200 + 360 = 560
    expect(detectZone(r, 350, 570)).toBe("bottom");
  });

  it("left/right take priority over top/bottom in corners", () => {
    // Top-left corner: both left (<20%) and top (<20%)
    // Left is checked first → returns "left"
    expect(detectZone(r, 110, 210)).toBe("left");
  });

  it("right takes priority over bottom in bottom-right corner", () => {
    expect(detectZone(r, 590, 590)).toBe("right");
  });

  it("respects exact edge boundary (20%)", () => {
    // At exactly 20% from left: relX = 0.2 → not < 0.2, so not "left"
    const x20 = 100 + 500 * 0.2; // = 200
    expect(detectZone(r, x20, 400)).toBe("center");
    // Just inside: relX = 0.199...
    expect(detectZone(r, x20 - 1, 400)).toBe("left");
  });

  it("works with custom edge threshold", () => {
    // 30% threshold → wider edge zones
    expect(detectZone(r, 200, 400, 0.3)).toBe("left"); // 20% from left, within 30% threshold
    expect(detectZone(r, 200, 400, 0.15)).toBe("center"); // 20% from left, outside 15% threshold
  });

  it("works with zero-origin rect", () => {
    const r0 = rect(0, 0, 1000, 1000);
    expect(detectZone(r0, 500, 500)).toBe("center");
    expect(detectZone(r0, 50, 500)).toBe("left");
    expect(detectZone(r0, 950, 500)).toBe("right");
    expect(detectZone(r0, 500, 50)).toBe("top");
    expect(detectZone(r0, 500, 950)).toBe("bottom");
  });

  it("works with small rect", () => {
    const small = rect(0, 0, 100, 100);
    // 20% of 100 = 20px
    expect(detectZone(small, 10, 50)).toBe("left");
    expect(detectZone(small, 90, 50)).toBe("right");
    expect(detectZone(small, 50, 50)).toBe("center");
  });
});

describe("zoneToDirection", () => {
  it("maps left to left", () => {
    expect(zoneToDirection("left")).toBe("left");
  });

  it("maps right to right", () => {
    expect(zoneToDirection("right")).toBe("right");
  });

  it("maps top to up", () => {
    expect(zoneToDirection("top")).toBe("up");
  });

  it("maps bottom to down", () => {
    expect(zoneToDirection("bottom")).toBe("down");
  });

  it("maps center to null (tabify, not split)", () => {
    expect(zoneToDirection("center")).toBeNull();
  });
});
