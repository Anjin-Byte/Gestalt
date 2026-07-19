// Keymap tests: the full physical binding table (a regression fence — WASD
// drift would be invisible to the type system) and the lookup's behavior on
// hostile codes.
import { KeyAction } from "voxel-web";
import { describe, expect, it } from "vitest";

import { actionFor } from "./keymap";

describe("actionFor", () => {
  it("binds the documented movement keys", () => {
    expect(actionFor("KeyW")).toBe(KeyAction.Forward);
    expect(actionFor("KeyS")).toBe(KeyAction.Back);
    expect(actionFor("KeyA")).toBe(KeyAction.Left);
    expect(actionFor("KeyD")).toBe(KeyAction.Right);
    expect(actionFor("KeyE")).toBe(KeyAction.Up);
    expect(actionFor("Space")).toBe(KeyAction.Up);
    expect(actionFor("KeyQ")).toBe(KeyAction.Down);
    expect(actionFor("ControlLeft")).toBe(KeyAction.Down);
    expect(actionFor("ShiftLeft")).toBe(KeyAction.Boost);
    expect(actionFor("ShiftRight")).toBe(KeyAction.Boost);
  });

  it("returns undefined for unbound keys", () => {
    expect(actionFor("KeyZ")).toBeUndefined();
    expect(actionFor("Escape")).toBeUndefined();
    expect(actionFor("")).toBeUndefined();
  });

  it("is case-sensitive on KeyboardEvent.code (no loose matching)", () => {
    expect(actionFor("keyw")).toBeUndefined();
    expect(actionFor("SPACE")).toBeUndefined();
  });

  it("does not fall through to Object prototype properties", () => {
    expect(actionFor("toString")).toBeUndefined();
    expect(actionFor("hasOwnProperty")).toBeUndefined();
    expect(actionFor("constructor")).toBeUndefined();
    expect(actionFor("__proto__")).toBeUndefined();
  });
});
