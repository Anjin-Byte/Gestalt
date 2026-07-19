// DOM input → data-plane setters. Events cross the boundary as scalars at
// event rate (docs/design/web-frontend-api.md §4); the shell decides what pointer
// motion means (drag-to-look) and which keys are movement intents.
import { type BrushTool, type KeyAction } from "voxel-web";

import { actionFor, radiusDeltaFor, toolFor } from "./keymap";
import { MAX_DPR } from "./render-protocol";

/** Where input scalars go: a render host, wherever the engine lives. */
export interface InputSink {
  key(action: KeyAction, down: boolean): void;
  pointerDelta(dx: number, dy: number): void;
  wheel(notches: number): void;
  /** One brush-stroke pointer event at device-pixel canvas coordinates, with
   * pen `pressure` in `[0, 1]` (1.0 for a mouse or a pen with no pressure). */
  brush(x: number, y: number, pressure: number): void;
  /** The sculpt pointer was released — end the stroke. */
  brushEnd(): void;
  /** The unpressed pointer moved: the hover pick for the cursor ring, at
   * device-pixel canvas coordinates. Negative coordinates mean the pointer
   * left the canvas (the ring deactivates). */
  hover(x: number, y: number): void;
  /** Undo the most recent stroke (`Cmd+Z`). */
  undo(): void;
  /** Re-apply the most recently undone stroke (`Shift+Cmd+Z`). */
  redo(): void;
  /** A tool hotkey (digits 1–7) — same path as clicking its button. */
  tool(tool: BrushTool): void;
  /** A radius nudge (`[` / `]`), ±1 voxel. */
  radiusDelta(delta: number): void;
  /** Alt held/released: the inverted tool arm (Inflate → deflate). */
  invert(active: boolean): void;
  /** The look-drag released — in orbit, commits the fling momentum. */
  lookEnd(): void;
  /** Alt + left-drag: pan the orbit pivot in the camera plane (pixels). */
  pan(dx: number, dy: number): void;
  /** Double-click: recentre the orbit pivot on the model. */
  resetPivot(): void;
}

/** Wheel notches per pixel of `deltaY` (a classic notch is ~100px). */
const NOTCH_PER_PX = 1 / 100;
/** Clamp on notches per event so free-spinning wheels don't warp the speed. */
const MAX_NOTCHES = 3;

// Gesture gate (docs/design/brush-editing/04-stroke-and-feel.md#click-vs-drag):
// stamps track lateral cursor travel in screen space — never event rate or
// world-hit drift (the "pillar on a click" failure, where Draw fills toward
// the camera and every event re-hits the geometry it just added). The press
// stamps once; each further stamp needs STAMP_SPACING_PX of net travel from
// the last one, so a tap or a jittering hold converges to exactly one stamp
// while a drag spaces stamps along its path. The threshold is CSS px (cursor
// motion is a CSS-px phenomenon, stable across display density and zoom);
// stamp density along the stroke is owned by the kernel's world-space
// resample — this gate only decides when the shell asks for a stamp at all.
const STAMP_SPACING_PX = 8;

/** Wires canvas/window listeners to the sink's input setters. */
export function attachInput(canvas: HTMLCanvasElement, sink: InputSink): void {
  let looking = false;
  let sculpting = false;
  let stampX = 0; // last emitted stamp, CSS px
  let stampY = 0;
  let lastHoverX = -1; // last hover sent, device px (dedupe)
  let lastHoverY = -1;
  const stampAt = (e: PointerEvent): void => {
    const dpr = Math.min(window.devicePixelRatio, MAX_DPR);
    // A mouse reports pressure 0 on move and 0.5 on button; a pen reports its
    // real force. Treat any non-pen device (or a 0) as full pressure so a mouse
    // paints at full strength.
    const pressure = e.pointerType === "pen" && e.pressure > 0 ? e.pressure : 1;
    sink.brush(e.offsetX * dpr, e.offsetY * dpr, pressure);
    stampX = e.offsetX;
    stampY = e.offsetY;
  };

  canvas.addEventListener("pointerdown", (e) => {
    canvas.setPointerCapture(e.pointerId);
    if (e.button === 2) {
      // Right button sculpts (matching the native viewer). The press stamps
      // once; further stamps require real cursor travel (the gesture gate).
      sculpting = true;
      stampAt(e);
      return;
    }
    canvas.classList.add("looking");
    looking = true;
  });
  const release = (e: PointerEvent) => {
    if (canvas.hasPointerCapture(e.pointerId)) {
      canvas.releasePointerCapture(e.pointerId);
    }
    canvas.classList.remove("looking");
    if (looking) {
      looking = false;
      sink.lookEnd(); // in orbit, the release commits the fling
    }
    if (sculpting) {
      sculpting = false;
      sink.brushEnd();
    }
  };
  canvas.addEventListener("pointerup", release);
  canvas.addEventListener("pointercancel", release);
  canvas.addEventListener("contextmenu", (e) => {
    e.preventDefault(); // the right button is the sculpt tool
  });
  canvas.addEventListener("dblclick", () => {
    sink.resetPivot(); // undo any Alt-drag pivot pan
  });
  canvas.addEventListener("pointermove", (e) => {
    if (looking) {
      // Alt + left-drag pans the orbit pivot instead of rotating (the brush's
      // Alt-deflate arm rides the right button, so the two never collide).
      if (e.altKey) {
        sink.pan(e.movementX, e.movementY);
      } else {
        sink.pointerDelta(e.movementX, e.movementY);
      }
    } else if (sculpting) {
      if (Math.hypot(e.offsetX - stampX, e.offsetY - stampY) >= STAMP_SPACING_PX) {
        stampAt(e);
      }
    } else {
      // No buttons: the hover pick for the cursor ring (µs-class kernel-side;
      // duplicate positions are skipped).
      const dpr = Math.min(window.devicePixelRatio, MAX_DPR);
      const hx = e.offsetX * dpr;
      const hy = e.offsetY * dpr;
      if (hx !== lastHoverX || hy !== lastHoverY) {
        lastHoverX = hx;
        lastHoverY = hy;
        sink.hover(hx, hy);
      }
    }
  });
  canvas.addEventListener("pointerleave", () => {
    lastHoverX = -1;
    lastHoverY = -1;
    sink.hover(-1, -1); // the ring deactivates off-canvas
  });

  canvas.addEventListener(
    "wheel",
    (e) => {
      e.preventDefault(); // page zoom/scroll never fights the speed control
      const notches = -e.deltaY * NOTCH_PER_PX;
      sink.wheel(Math.max(-MAX_NOTCHES, Math.min(MAX_NOTCHES, notches)));
    },
    { passive: false },
  );

  window.addEventListener("keydown", (e) => {
    handleKey(e, true);
  });
  window.addEventListener("keyup", (e) => {
    handleKey(e, false);
  });
  // A window blur can swallow the Alt keyup; never leave the inverted arm
  // stuck on.
  window.addEventListener("blur", () => {
    sink.invert(false);
  });

  function handleKey(e: KeyboardEvent, down: boolean): void {
    // The one deliberate metaKey exception (docs/design/brush-editing/04):
    // Cmd+Z / Shift+Cmd+Z is stroke undo/redo. preventDefault suppresses the
    // browser's own undo; there are no text-entry fields to be robbed of it.
    // Auto-repeat deliberately passes — holding the chord scrubs history.
    if (e.metaKey && e.code === "KeyZ") {
      if (down) {
        e.preventDefault();
        if (e.shiftKey) {
          sink.redo();
        } else {
          sink.undo();
        }
      }
      return;
    }
    if (e.metaKey) {
      return; // never eat browser shortcuts
    }
    // Form controls (the HUD selects) keep their native keyboard behavior.
    if (e.target instanceof HTMLSelectElement || e.target instanceof HTMLInputElement) {
      return;
    }
    // Alt is the inverted-tool modifier (Inflate → deflate); preventDefault
    // keeps the browser's menu-bar focus off it.
    if (e.code === "AltLeft" || e.code === "AltRight") {
      e.preventDefault();
      if (!e.repeat) {
        sink.invert(down);
      }
      return;
    }
    if (down) {
      // Tool hotkeys (1–7) and the [ / ] radius nudge are shell-level: the
      // same code path as the HUD buttons, one control-plane call.
      const tool = toolFor(e.code);
      if (tool !== undefined) {
        if (!e.repeat) {
          sink.tool(tool);
        }
        return;
      }
      const nudge = radiusDeltaFor(e.code);
      if (nudge !== undefined) {
        sink.radiusDelta(nudge); // repeats pass — hold to resize
        return;
      }
    }
    const action = actionFor(e.code);
    if (action === undefined) {
      return;
    }
    if (e.code === "Space") {
      e.preventDefault(); // page scroll
    }
    if (!e.repeat) {
      sink.key(action, down);
    }
  }
}
