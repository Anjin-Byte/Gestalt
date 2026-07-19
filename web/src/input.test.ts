// @vitest-environment happy-dom
// attachInput tests: the pointer state machine (look vs sculpt), device-pixel
// scaling, wheel clamping, and the keyboard gates (repeat, meta, form
// controls). The sink is a recorder — the oracle is the exact call sequence.
import { BrushTool, KeyAction } from "voxel-web";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { attachInput, type InputSink } from "./input";

type Call =
  | { readonly kind: "key"; readonly action: KeyAction; readonly down: boolean }
  | { readonly kind: "pointerDelta"; readonly dx: number; readonly dy: number }
  | { readonly kind: "wheel"; readonly notches: number }
  | { readonly kind: "brush"; readonly x: number; readonly y: number }
  | { readonly kind: "brushEnd" }
  | { readonly kind: "undo" }
  | { readonly kind: "redo" }
  | { readonly kind: "hover"; readonly x: number; readonly y: number }
  | { readonly kind: "tool"; readonly tool: BrushTool }
  | { readonly kind: "radiusDelta"; readonly delta: number }
  | { readonly kind: "invert"; readonly active: boolean }
  | { readonly kind: "lookEnd" }
  | { readonly kind: "pan"; readonly dx: number; readonly dy: number }
  | { readonly kind: "resetPivot" };

function makeSink(): { sink: InputSink; calls: Call[] } {
  const calls: Call[] = [];
  return {
    calls,
    sink: {
      key: (action, down) => calls.push({ kind: "key", action, down }),
      pointerDelta: (dx, dy) => calls.push({ kind: "pointerDelta", dx, dy }),
      wheel: (notches) => calls.push({ kind: "wheel", notches }),
      brush: (x, y) => calls.push({ kind: "brush", x, y }),
      brushEnd: () => calls.push({ kind: "brushEnd" }),
      undo: () => calls.push({ kind: "undo" }),
      redo: () => calls.push({ kind: "redo" }),
      hover: (x, y) => calls.push({ kind: "hover", x, y }),
      tool: (tool) => calls.push({ kind: "tool", tool }),
      radiusDelta: (delta) => calls.push({ kind: "radiusDelta", delta }),
      invert: (active) => calls.push({ kind: "invert", active }),
      lookEnd: () => calls.push({ kind: "lookEnd" }),
      pan: (dx, dy) => calls.push({ kind: "pan", dx, dy }),
      resetPivot: () => calls.push({ kind: "resetPivot" }),
    },
  };
}

interface PointerInit extends PointerEventInit {
  readonly offsetX?: number;
  readonly offsetY?: number;
  readonly movementX?: number;
  readonly movementY?: number;
}

/** Dispatches a pointer event with the offset/movement fields the DOM
 * computes in real browsers (constructor init cannot set them). */
function firePointer(target: Element, type: string, init: PointerInit = {}): PointerEvent {
  const { offsetX = 0, offsetY = 0, movementX = 0, movementY = 0, ...rest } = init;
  const e = new PointerEvent(type, { bubbles: true, cancelable: true, ...rest });
  Object.defineProperty(e, "offsetX", { value: offsetX });
  Object.defineProperty(e, "offsetY", { value: offsetY });
  Object.defineProperty(e, "movementX", { value: movementX });
  Object.defineProperty(e, "movementY", { value: movementY });
  target.dispatchEvent(e);
  return e;
}

function fireWheel(target: Element, deltaY: number): WheelEvent {
  const e = new WheelEvent("wheel", { deltaY, cancelable: true });
  target.dispatchEvent(e);
  return e;
}

function fireKey(
  target: EventTarget,
  type: "keydown" | "keyup",
  init: KeyboardEventInit,
): KeyboardEvent {
  const e = new KeyboardEvent(type, { bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(e);
  return e;
}

/** A canvas with deterministic pointer-capture bookkeeping. */
function makeCanvas(): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  const captured = new Set<number>();
  canvas.setPointerCapture = vi.fn((id: number) => captured.add(id));
  canvas.hasPointerCapture = vi.fn((id: number) => captured.has(id));
  canvas.releasePointerCapture = vi.fn((id: number) => captured.delete(id));
  document.body.appendChild(canvas);
  return canvas;
}

function setDpr(value: number): void {
  Object.defineProperty(window, "devicePixelRatio", { value, configurable: true });
}

beforeEach(() => {
  document.body.innerHTML = "";
  setDpr(1);
});

describe("attachInput look (left drag)", () => {
  it("streams movement deltas only while the left button is held", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);

    firePointer(canvas, "pointermove", { movementX: 9, movementY: 9 }); // before press → hover
    firePointer(canvas, "pointerdown", { button: 0, pointerId: 1 });
    expect(canvas.classList.contains("looking")).toBe(true);
    expect(canvas.setPointerCapture).toHaveBeenCalledWith(1);
    firePointer(canvas, "pointermove", { movementX: 3, movementY: -2 });
    firePointer(canvas, "pointermove", { movementX: -1, movementY: 4 });
    firePointer(canvas, "pointerup", { button: 0, pointerId: 1 });
    expect(canvas.classList.contains("looking")).toBe(false);
    expect(canvas.releasePointerCapture).toHaveBeenCalledWith(1);
    firePointer(canvas, "pointermove", { movementX: 8, movementY: 8 }); // after release → hover

    // Button-less moves are hover picks, not look deltas; while looking, only
    // deltas flow (no hover noise mid-look).
    expect(calls.filter((c) => c.kind === "pointerDelta")).toEqual([
      { kind: "pointerDelta", dx: 3, dy: -2 },
      { kind: "pointerDelta", dx: -1, dy: 4 },
    ]);
    expect(calls.filter((c) => c.kind === "hover")).toHaveLength(1); // deduped same-spot moves
  });

  it("ends a look on pointercancel too", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    firePointer(canvas, "pointerdown", { button: 0, pointerId: 1 });
    firePointer(canvas, "pointercancel", { pointerId: 1 });
    firePointer(canvas, "pointermove", { movementX: 5 });
    // The cancel still ends the look properly — committing the fling.
    expect(calls.filter((c) => c.kind !== "hover")).toEqual([{ kind: "lookEnd" }]);
    expect(canvas.classList.contains("looking")).toBe(false);
  });

  it("commits the fling with lookEnd when the look-drag releases", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    firePointer(canvas, "pointerdown", { button: 0, pointerId: 1 });
    firePointer(canvas, "pointermove", { movementX: 6, movementY: 1 });
    firePointer(canvas, "pointerup", { button: 0, pointerId: 1 });
    expect(calls.filter((c) => c.kind === "lookEnd")).toHaveLength(1);
    // A stray second release must not double-commit.
    firePointer(canvas, "pointerup", { button: 0, pointerId: 1 });
    expect(calls.filter((c) => c.kind === "lookEnd")).toHaveLength(1);
  });

  it("routes an alt-held look-drag to pivot pan instead of rotation", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    firePointer(canvas, "pointerdown", { button: 0, pointerId: 1 });
    firePointer(canvas, "pointermove", { movementX: 7, movementY: -3, altKey: true });
    firePointer(canvas, "pointermove", { movementX: 2, movementY: 5 }); // alt released mid-drag
    firePointer(canvas, "pointerup", { button: 0, pointerId: 1 });
    expect(calls.filter((c) => c.kind === "pan")).toEqual([{ kind: "pan", dx: 7, dy: -3 }]);
    expect(calls.filter((c) => c.kind === "pointerDelta")).toEqual([
      { kind: "pointerDelta", dx: 2, dy: 5 },
    ]);
  });

  it("recentres the pivot on double-click", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    canvas.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    expect(calls).toEqual([{ kind: "resetPivot" }]);
  });
});

describe("attachInput sculpt (right drag)", () => {
  it("brushes at device-pixel coordinates from press through drag, then ends the stroke", () => {
    setDpr(2);
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);

    firePointer(canvas, "pointerdown", { button: 2, pointerId: 4, offsetX: 10, offsetY: 20 });
    firePointer(canvas, "pointermove", { offsetX: 30, offsetY: 20 }); // real travel: 20 CSS px
    firePointer(canvas, "pointerup", { button: 2, pointerId: 4 });

    expect(calls).toEqual([
      { kind: "brush", x: 20, y: 40 },
      { kind: "brush", x: 60, y: 40 },
      { kind: "brushEnd" },
    ]);
    expect(canvas.classList.contains("looking")).toBe(false); // sculpting is not looking
  });

  it("clamps a 3× display to the shared MAX_DPR of 2", () => {
    setDpr(3);
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    firePointer(canvas, "pointerdown", { button: 2, pointerId: 1, offsetX: 100, offsetY: 50 });
    expect(calls[0]).toEqual({ kind: "brush", x: 200, y: 100 });
  });

  it("emits exactly one brushEnd per stroke, even with stray releases", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    firePointer(canvas, "pointerdown", { button: 2, pointerId: 1, offsetX: 1, offsetY: 1 });
    firePointer(canvas, "pointerup", { button: 2, pointerId: 1 });
    firePointer(canvas, "pointerup", { button: 2, pointerId: 1 }); // stray double release
    expect(calls.filter((c) => c.kind === "brushEnd")).toHaveLength(1);
  });

  it("never emits brushEnd for a pure look drag", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    firePointer(canvas, "pointerdown", { button: 0, pointerId: 1 });
    firePointer(canvas, "pointerup", { button: 0, pointerId: 1 });
    expect(calls.filter((c) => c.kind === "brushEnd")).toHaveLength(0);
  });

  it("suppresses the context menu so the right button stays a tool", () => {
    const canvas = makeCanvas();
    attachInput(canvas, makeSink().sink);
    const e = new MouseEvent("contextmenu", { cancelable: true });
    canvas.dispatchEvent(e);
    expect(e.defaultPrevented).toBe(true);
  });
});

describe("attachInput gesture gate (tap vs drag)", () => {
  // The pillar bug's oracle: stamps track lateral cursor travel, never event
  // rate (docs/design/brush-editing/04-stroke-and-feel.md#click-vs-drag).
  it("a jittery tap places exactly one stamp, at the press point", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);

    firePointer(canvas, "pointerdown", { button: 2, pointerId: 1, offsetX: 100, offsetY: 100 });
    // Hand jitter / high-Hz events: net travel never reaches STAMP_SPACING_PX.
    firePointer(canvas, "pointermove", { offsetX: 101, offsetY: 100 });
    firePointer(canvas, "pointermove", { offsetX: 99, offsetY: 101 });
    firePointer(canvas, "pointermove", { offsetX: 102, offsetY: 102 });
    firePointer(canvas, "pointermove", { offsetX: 97, offsetY: 100 });
    firePointer(canvas, "pointerup", { button: 2, pointerId: 1 });

    expect(calls).toEqual([{ kind: "brush", x: 100, y: 100 }, { kind: "brushEnd" }]);
  });

  it("a drag spaces stamps by cursor travel, not by move-event count", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);

    firePointer(canvas, "pointerdown", { button: 2, pointerId: 1, offsetX: 0, offsetY: 0 });
    firePointer(canvas, "pointermove", { offsetX: 2, offsetY: 0 }); // < spacing
    firePointer(canvas, "pointermove", { offsetX: 5, offsetY: 0 }); // < spacing
    firePointer(canvas, "pointermove", { offsetX: 7, offsetY: 0 }); // < spacing
    firePointer(canvas, "pointermove", { offsetX: 8, offsetY: 0 }); // spacing crossed
    firePointer(canvas, "pointermove", { offsetX: 9, offsetY: 0 });
    firePointer(canvas, "pointermove", { offsetX: 12, offsetY: 0 });
    firePointer(canvas, "pointermove", { offsetX: 15, offsetY: 0 });
    firePointer(canvas, "pointermove", { offsetX: 16, offsetY: 0 }); // spacing crossed again
    firePointer(canvas, "pointerup", { button: 2, pointerId: 1 });

    expect(calls).toEqual([
      { kind: "brush", x: 0, y: 0 },
      { kind: "brush", x: 8, y: 0 },
      { kind: "brush", x: 16, y: 0 },
      { kind: "brushEnd" },
    ]);
  });

  it("jitter around the last stamp mid-drag emits nothing until travel resumes", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);

    firePointer(canvas, "pointerdown", { button: 2, pointerId: 1, offsetX: 0, offsetY: 0 });
    firePointer(canvas, "pointermove", { offsetX: 20, offsetY: 0 }); // stamp
    // A jittering hold: net displacement from the last stamp stays small, so
    // accumulated path length never converts into stamps (no slow pillar).
    firePointer(canvas, "pointermove", { offsetX: 18, offsetY: 0 });
    firePointer(canvas, "pointermove", { offsetX: 22, offsetY: 0 });
    firePointer(canvas, "pointermove", { offsetX: 19, offsetY: 1 });
    firePointer(canvas, "pointermove", { offsetX: 21, offsetY: -1 });
    firePointer(canvas, "pointermove", { offsetX: 40, offsetY: 0 }); // stamp
    firePointer(canvas, "pointerup", { button: 2, pointerId: 1 });

    expect(calls.filter((c) => c.kind === "brush")).toEqual([
      { kind: "brush", x: 0, y: 0 },
      { kind: "brush", x: 20, y: 0 },
      { kind: "brush", x: 40, y: 0 },
    ]);
  });

  it("each press resets the gate: a tap after a drag is still one stamp", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);

    firePointer(canvas, "pointerdown", { button: 2, pointerId: 1, offsetX: 0, offsetY: 0 });
    firePointer(canvas, "pointermove", { offsetX: 30, offsetY: 0 }); // a real drag
    firePointer(canvas, "pointerup", { button: 2, pointerId: 1 });
    firePointer(canvas, "pointerdown", { button: 2, pointerId: 1, offsetX: 200, offsetY: 200 });
    firePointer(canvas, "pointermove", { offsetX: 201, offsetY: 201 }); // jitter only
    firePointer(canvas, "pointerup", { button: 2, pointerId: 1 });

    expect(calls).toEqual([
      { kind: "brush", x: 0, y: 0 },
      { kind: "brush", x: 30, y: 0 },
      { kind: "brushEnd" },
      { kind: "brush", x: 200, y: 200 },
      { kind: "brushEnd" },
    ]);
  });
});

describe("attachInput wheel", () => {
  it("converts deltaY pixels to notches, clamped to ±3, and blocks page scroll", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    const e1 = fireWheel(canvas, -100); // one notch up
    fireWheel(canvas, 50); // half a notch down
    fireWheel(canvas, -1000); // free-spin: clamp to +3
    fireWheel(canvas, 1000); // clamp to -3
    expect(calls).toEqual([
      { kind: "wheel", notches: 1 },
      { kind: "wheel", notches: -0.5 },
      { kind: "wheel", notches: 3 },
      { kind: "wheel", notches: -3 },
    ]);
    expect(e1.defaultPrevented).toBe(true);
  });
});

describe("attachInput keyboard", () => {
  it("maps held movement keys to intents on both edges", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    fireKey(window, "keydown", { code: "KeyW" });
    fireKey(window, "keyup", { code: "KeyW" });
    fireKey(window, "keydown", { code: "ShiftLeft" });
    expect(calls).toEqual([
      { kind: "key", action: KeyAction.Forward, down: true },
      { kind: "key", action: KeyAction.Forward, down: false },
      { kind: "key", action: KeyAction.Boost, down: true },
    ]);
  });

  it("suppresses auto-repeat so held keys stay a single edge", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    fireKey(window, "keydown", { code: "KeyW" });
    fireKey(window, "keydown", { code: "KeyW", repeat: true });
    fireKey(window, "keydown", { code: "KeyW", repeat: true });
    expect(calls).toHaveLength(1);
  });

  it("never eats browser shortcuts (meta-key chords pass through)", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    const e = fireKey(window, "keydown", { code: "KeyW", metaKey: true });
    expect(calls).toEqual([]);
    expect(e.defaultPrevented).toBe(false);
  });

  it("Cmd+Z undoes and Shift+Cmd+Z redoes — the one deliberate meta exception", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    const undo = fireKey(window, "keydown", { code: "KeyZ", metaKey: true });
    const redo = fireKey(window, "keydown", { code: "KeyZ", metaKey: true, shiftKey: true });
    // The browser's own undo must not also fire.
    expect(undo.defaultPrevented).toBe(true);
    expect(redo.defaultPrevented).toBe(true);
    // Releases are inert; repeats scrub history deliberately.
    fireKey(window, "keyup", { code: "KeyZ", metaKey: true });
    fireKey(window, "keydown", { code: "KeyZ", metaKey: true, repeat: true });
    expect(calls).toEqual([{ kind: "undo" }, { kind: "redo" }, { kind: "undo" }]);
  });

  it("digits select tools, brackets nudge radius (repeats pass), Alt inverts", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    fireKey(window, "keydown", { code: "Digit3" });
    fireKey(window, "keydown", { code: "Digit3", repeat: true }); // tool repeat swallowed
    fireKey(window, "keydown", { code: "Digit7" });
    fireKey(window, "keydown", { code: "BracketRight" });
    fireKey(window, "keydown", { code: "BracketRight", repeat: true }); // hold to resize
    fireKey(window, "keydown", { code: "BracketLeft" });
    const alt = fireKey(window, "keydown", { code: "AltLeft" });
    fireKey(window, "keyup", { code: "AltLeft" });
    expect(alt.defaultPrevented).toBe(true); // no browser menu focus
    expect(calls).toEqual([
      { kind: "tool", tool: BrushTool.Clay },
      { kind: "tool", tool: BrushTool.Paint },
      { kind: "radiusDelta", delta: 1 },
      { kind: "radiusDelta", delta: 1 },
      { kind: "radiusDelta", delta: -1 },
      { kind: "invert", active: true },
      { kind: "invert", active: false },
    ]);
  });

  it("a window blur releases a stuck Alt", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    fireKey(window, "keydown", { code: "AltRight" });
    window.dispatchEvent(new Event("blur"));
    expect(calls).toEqual([
      { kind: "invert", active: true },
      { kind: "invert", active: false },
    ]);
  });
});

describe("attachInput hover (the cursor ring pick)", () => {
  it("forwards button-less moves at device pixels, deduped, and deactivates on leave", () => {
    setDpr(2);
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    firePointer(canvas, "pointermove", { offsetX: 10, offsetY: 20 });
    firePointer(canvas, "pointermove", { offsetX: 10, offsetY: 20 }); // duplicate: skipped
    firePointer(canvas, "pointermove", { offsetX: 11, offsetY: 20 });
    firePointer(canvas, "pointerleave");
    expect(calls).toEqual([
      { kind: "hover", x: 20, y: 40 },
      { kind: "hover", x: 22, y: 40 },
      { kind: "hover", x: -1, y: -1 },
    ]);
  });

  it("never hovers while sculpting (the stroke owns the pointer)", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    firePointer(canvas, "pointerdown", { button: 2, pointerId: 1, offsetX: 0, offsetY: 0 });
    firePointer(canvas, "pointermove", { offsetX: 30, offsetY: 0 });
    expect(calls.filter((c) => c.kind === "hover")).toEqual([]);
  });

  it("leaves form controls their native keyboard behavior", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    const select = document.createElement("select");
    const input = document.createElement("input");
    document.body.append(select, input);
    fireKey(select, "keydown", { code: "KeyW" });
    fireKey(input, "keydown", { code: "KeyS" });
    expect(calls).toEqual([]);
  });

  it("prevents Space from scrolling the page while still rising", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    const e = fireKey(window, "keydown", { code: "Space" });
    expect(e.defaultPrevented).toBe(true);
    expect(calls).toEqual([{ kind: "key", action: KeyAction.Up, down: true }]);
  });

  it("ignores unbound keys, including prototype-property names", () => {
    const canvas = makeCanvas();
    const { sink, calls } = makeSink();
    attachInput(canvas, sink);
    fireKey(window, "keydown", { code: "KeyZ" });
    fireKey(window, "keydown", { code: "toString" });
    fireKey(window, "keydown", { code: "" });
    expect(calls).toEqual([]);
  });
});
