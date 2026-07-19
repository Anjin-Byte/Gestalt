// The physical keymap is the shell's to own; the kernel only sees KeyAction
// intents (docs/design/web-frontend-api.md §4).
import { BrushTool, KeyAction } from "voxel-web";

const BINDINGS = {
  KeyW: KeyAction.Forward,
  KeyS: KeyAction.Back,
  KeyA: KeyAction.Left,
  KeyD: KeyAction.Right,
  KeyE: KeyAction.Up,
  Space: KeyAction.Up,
  KeyQ: KeyAction.Down,
  ControlLeft: KeyAction.Down,
  ShiftLeft: KeyAction.Boost,
  ShiftRight: KeyAction.Boost,
} as const satisfies Record<string, KeyAction>;

/** The movement intent bound to a `KeyboardEvent.code`, if any. */
export function actionFor(code: string): KeyAction | undefined {
  return Object.hasOwn(BINDINGS, code)
    ? BINDINGS[code as keyof typeof BINDINGS]
    : undefined;
}

// Tool hotkeys are digits in the palette's display order (Stage D). The
// design's ZBrush letter set (B/E/P/C/S/F/I) collides with the WASD-fly
// movement map this file also owns (S = Back, E = Up), so digits — the
// standard editor fallback — carry the tools instead; tool keys are
// shell-level (the same code path as clicking the button, one `set_brush`).
const TOOL_BINDINGS = {
  Digit1: BrushTool.Draw,
  Digit2: BrushTool.Erase,
  Digit3: BrushTool.Clay,
  Digit4: BrushTool.Smooth,
  Digit5: BrushTool.Flatten,
  Digit6: BrushTool.Inflate,
  Digit7: BrushTool.Paint,
} as const satisfies Record<string, BrushTool>;

/** The brush tool bound to a `KeyboardEvent.code`, if any. */
export function toolFor(code: string): BrushTool | undefined {
  return Object.hasOwn(TOOL_BINDINGS, code)
    ? TOOL_BINDINGS[code as keyof typeof TOOL_BINDINGS]
    : undefined;
}

/** The radius nudge bound to a code: `[` shrinks, `]` grows (the native
 * viewer's precedent). Auto-repeat deliberately passes — hold to resize. */
export function radiusDeltaFor(code: string): number | undefined {
  if (code === "BracketLeft") {
    return -1;
  }
  if (code === "BracketRight") {
    return 1;
  }
  return undefined;
}
