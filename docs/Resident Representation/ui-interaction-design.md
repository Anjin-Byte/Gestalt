# UI Interaction Design

**Type:** spec
**Status:** stale
**Date:** 2026-03-21

---

## Philosophy

> Every pixel is either information or structure. Nothing else earns space on screen.

This document defines the interaction design language for the Gestalt testbed UI. The primary reference is Blender's Properties system — the most successful example of a high-density, deeply-nested developer/artist tool that never feels disorganized. The principles are not aesthetic preferences. Each one exists because it solves a specific usability problem at high information density.

---

## Core Principles

### 1. No decorative chrome

Visual elements are either data or delimiters. Background fills, drop shadows, and border-radius are not used to create visual interest — only to separate genuinely distinct surfaces (sidebar, panel area, viewport). Within a panel, sections are separated by spatial grouping and a single 1px hairline, not by nested cards with fills.

**Corollary:** If you can't answer "what information does this border communicate?", remove it.

### 2. Spatial grammar over visual hierarchy

Nesting is expressed through indentation and vertical proximity, not through background fills or elevation. A sub-section sits 8px further right than its parent label. A sub-sub-section another 8px. No fill, no shadow, no border-radius stack. This is how Blender manages properties panels with five levels of nesting that remain readable — the depth is spatial, invisible to the eye, read by position.

### 3. Every adjustable value is an interactive surface

Read-only displays and interactive controls are visually identical except for their hover cursor. There are no separate slider widgets taking vertical space. The value text itself is the interactive element. This is the scrub field pattern (see §Components).

### 4. Consistent interaction model across all numeric types

Drag left/right to decrease/increase. Click to type. Double-click to reset to default. This applies to every numeric value in every panel. Once a user learns it on one field, they have learned it everywhere. Blender users don't think "is this a slider or a number input?" — the question doesn't exist because there is only one kind of numeric field.

### 5. Right-click surfaces the full capability

The display surface is minimal by design. Right-click exposes: copy value, reset to default, copy data path (for automation). The primary view stays uncluttered; power operations live one gesture away.

### 6. Density through rhythm, not cramming

High information density is achieved by consistent 4px internal gaps and 10–12px section gaps — not by reducing font size below legibility. The rhythm never breaks. 11px for keys and labels, 11px monospace for values, no exceptions. When spacing is consistent, the eye processes rows as a unit rather than reading each element individually.

### 7. Context-sensitive status

The status bar at the bottom of the viewport reflects what interaction is available based on cursor context. "Drag to scrub · Click to type · Right-click to reset" when hovering a scrub field. "Click to copy" on a prop row. This is how Blender teaches its interaction model passively — not documentation, not tooltips, but a persistent one-line readout that is always accurate and always current.

---

## Components

### ScrubField

The primary numeric input component. Replaces the `<Slider>` component and all `<input type="range">` in panel contexts.

```
┌─────────────────────────────────────┐
│  Label                       1.40   │  ← hover: cursor becomes ew-resize
└─────────────────────────────────────┘
         ↑                       ↑
    left-aligned 11px      right-aligned 11px mono
    muted key color        bright value color
```

**States:**

| State | Appearance | Interaction |
|---|---|---|
| Rest | Label left, value right, no chrome | — |
| Hover | Subtle background tint, `cursor: ew-resize` | Drag = scrub |
| Dragging | Active tint, value updates live | Mouse delta × step |
| Editing | Value becomes `<input type="text">` inline | Type + Enter to commit, Escape to cancel |
| Reset pending | Brief flash on double-click | Revert to `defaultValue` |

**Props:**
```typescript
type ScrubFieldProps = {
  label: string;
  value: number;
  defaultValue: number;
  min?: number;
  max?: number;
  step?: number;       // default: 0.01
  decimals?: number;   // default: 2
  unit?: string;       // appended to value: "128 MB", "256", "1.40"
  onValueChange: (v: number) => void;
};
```

**Scrub sensitivity:** `delta_px × step`. Holding Shift during drag multiplies by 0.1 for fine control. Holding Ctrl multiplies by 10 for coarse.

**Implementation notes:**
- Capture `pointermove` on `pointerdown` + `setPointerCapture` so drag continues outside the element
- Commit on `pointerup`; cancel on `Escape` during keyboard edit
- `touch-action: none` to prevent scroll interference on touch devices

---

### PropRow

Read-only key/value display with clipboard affordance.

```
  Renderer                  WebGPU  [⎘]   ← copy icon fades in on hover
  Max invocations            1,024
  Max storage               128 MB
```

**Interaction:**
- Hover row → copy icon fades in (opacity 0 → 1, 100ms)
- Click copy icon → `navigator.clipboard.writeText(value)` → icon swaps to `Check`, green tint, 1.4s → fades back
- Right-click row → context menu: "Copy value", "Copy as JSON"

**Note:** The copy icon appears on the right, before the value, so it doesn't shift the value's position on hover. The value stays right-aligned and stable.

---

### Section

Collapsible panel section with persistent state.

```
▶ DEVICE                     ← closed: chevron points right
▼ DEVICE                     ← open: chevron points down, rotated 90°
  ├── content row
  └── content row
```

**Behavior:**
- Open/closed state persisted to `localStorage` keyed by `sectionId`
- `transition:slide` on the body (140ms, `easing: cubicOut`)
- The trigger button is keyboard-focusable; Enter/Space toggle
- Section header font: 11px, uppercase, 0.05em letter-spacing, neutral `oklch(0.75 0.005 250)`
- The chevron is the only color change on toggle (dims open, brightens closed) — the title does not change color

**Pinning (Phase 4):** Sections will support a pin affordance (right-click header → "Pin to top"). Pinned sections float above unpin sections and persist across panel tab switches. Stored in `localStorage` as a separate key.

---

### Checkbox row

```
□  Wireframe          ← unchecked: near-invisible border, no fill
■  Axes               ← checked: primary blue fill, white checkmark
```

The label and checkbox are both part of a `<label>` element — the entire row is the click target. No separate hit area. Padding: 3px vertical for touch comfort.

---

### Status Bar

Single fixed line at the bottom edge of the viewport. Updates on `mousemove` based on the element under the cursor.

```
┌──────────────────────────────────────────────────────────────────────┐
│  Drag ← → to scrub  ·  Click to edit  ·  Double-click to reset      │
└──────────────────────────────────────────────────────────────────────┘
```

**Implementation:** A Svelte store `statusHint = writable<string>("")`. Components set it on `mouseenter` / clear on `mouseleave`. The status bar reads `$statusHint`. No DOM querying.

Hints by component:
- `ScrubField`: "Drag ← → to scrub · Click to type · Double-click to reset"
- `PropRow` (hovering): "Click [⎘] to copy"
- `Section trigger`: "Click to collapse · State saved"
- Viewport canvas: FPS / frame time (replaces the inline overlay)

---

## What We Are Not Doing

**Tooltips on hover delay.** Blender uses them; we won't. The status bar serves the same purpose without the 500ms latency and without obscuring content.

**Animated transitions on values.** Smooth number interpolation adds latency between reality and display. Values update immediately or not at all.

**Modal dialogs for any configuration.** Everything is in-panel, collapsible, and in-place. No modal for settings, no popup for confirmation (destructive actions use undo instead).

**Dark/light mode toggle.** The testbed is a developer tool. It is always dark. The light-mode tokens in `app.css` exist as CSS fallback, not as a switchable theme.

**Icon labels in the sidebar.** Icons are identified by tooltip on hover and by the status bar. Adding text labels under each icon breaks the spatial grammar by introducing a second visual tier in the sidebar column. The 56px sidebar width is intentional — it forces icon-only communication, which works once the user has mapped the icons (three sessions).

---

## Visual Reference Hierarchy

```
Panel area background      oklch(0.18 0.015 250)   ← base surface
  Section separator        oklch(1 0 0 / 5%)        ← hairline only
  Key text                 oklch(0.52 0.01 250)     ← recedes
  Value text               oklch(0.82 0.005 250)    ← comes forward
  Active/interactive       oklch(0.72 0.11 250)     ← primary blue, used ONLY for interactive state
  Hover tint               oklch(1 0 0 / 4%)        ← scrub field hover, row hover
  Section header           oklch(0.75 0.005 250)    ← structural, not decorative
  Muted / disabled         oklch(0.45 0.008 250)    ← placeholder, inactive
```

One chroma value (`0.11`) used for one semantic purpose (interactive / active). Everything else is a lightness step on hue 250. This is the palette.

---

## Implementation Priority

| Component | Status | Notes |
|---|---|---|
| `Section.svelte` | ✅ Done | Chevron, slide transition, localStorage |
| `PropRow.svelte` | ✅ Done | Hover copy, Check confirmation |
| `Slider.svelte` | ✅ Done | Inline value — to be replaced by ScrubField |
| `ScrubField.svelte` | 🔲 Next | Replaces Slider entirely |
| `StatusBar.svelte` | 🔲 Phase 4 | Svelte store hint system |
| Section pinning | 🔲 Phase 4 | Right-click → pin, localStorage |
| Right-click context menu | 🔲 Phase 4 | Copy, reset to default, copy path |
