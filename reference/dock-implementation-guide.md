# Dock System Implementation Guide

**Type:** reference
**Status:** current
**Date:** 2026-03-22
**Scope:** Lessons learned from studying Blender, dockview, and Godot. Defines the architecture for Phi's dock system.

---

## The Core Lesson

We tried to build a vertex-edge-area graph with CSS Grid rendering. It didn't work because:

1. CSS Grid percentages don't compose well with interactive resize — you can't efficiently update tracks during drag
2. The graph model is complex to get right (Blender has 20+ years of edge case fixes)
3. Svelte's `$state` proxy wrappers break `structuredClone`
4. Reactivity and manual graph mutation are fundamentally at odds

**dockview solves this with a simpler, proven architecture** — and it powers VS Code, Azure Data Studio, and hundreds of production apps.

---

## The Architecture: Three Layers

### Layer 1 — Splitview (1D constraint solver)

The atomic primitive. A single-axis layout of N views separated by sashes (splitter handles).

**Data model:**
```
ViewItem {
  size: number          // current pixel size
  minimumSize: number
  maximumSize: number
  visible: boolean
  cachedVisibleSize?: number  // remembers size when hidden
  priority: Low | Normal | High
  element: HTMLElement
}

Splitview {
  viewItems: ViewItem[]
  sashes: SashElement[]
  size: number          // total container size in pixels
  proportions: number[] // saved fractions for container resize
  orientation: H | V
}
```

**The constraint solver** (`resize(sashIndex, delta, snapshotSizes)`):
1. Divide views into "up" group (before sash) and "down" group (after sash)
2. Sort each group by priority (High first, Low last)
3. Compute feasible delta range from min/max constraints of both sides
4. Clamp delta to feasible range
5. Distribute clamped delta to up-group: walk in priority order, clamp each to [min, max], carry remainder
6. Distribute the negative to down-group: same greedy walk
7. Run `distributeEmptySpace()` to fix rounding errors

**Key patterns from dockview's splitview.ts:**
- **Snapshot-based dragging** — delta is always from drag-start state, never incremental. Prevents drift.
- **`distributeEmptySpace()`** — runs after every operation. Guarantees `sum(sizes) == containerSize`.
- **Proportional resize is a saved snapshot** — only consulted during container resize (`ResizeObserver`). Interactive drag is purely pixel-based. Proportions are re-saved after drag ends.
- **DOM-based sizing** — each view gets `element.style.width/height = size + 'px'`. No CSS Grid. No percentage calculations. Just pixels.

### Layer 2 — Gridview (2D from nested 1D)

A recursive tree where every internal node IS a Splitview, and orientation alternates at each depth.

```
BranchNode implements IView {
  splitview: Splitview
  children: (BranchNode | LeafNode)[]
  orientation: H | V
}

LeafNode implements IView {
  view: IGridView  // the actual panel
}
```

**The key trick:** `BranchNode` implements `IView` (has `minimumSize`, `maximumSize`, `layout()`). A Splitview's child can be another Splitview. The `IView` interface is the recursive composition point.

**Adding a panel (direction + target):**
1. `getRelativeLocation(direction, targetLocation)` — converts "left of panel X" into a tree path
2. If orientations match (e.g., dropping left at a horizontal level): insert as sibling
3. If orientations don't match: push location one level deeper → triggers tree restructuring (the target leaf is replaced by a new branch containing the old leaf + new leaf)

**Removing a panel:**
1. Remove leaf from parent branch
2. If parent has only 1 child remaining: collapse the branch (promote the child up)
3. Sizes are preserved through the restructuring

**Serialization:**
```json
{
  "root": {
    "type": "branch",
    "size": 800,
    "data": [
      { "type": "leaf", "size": 240, "data": { "id": "group-1", "views": ["scene"], "activeView": "scene" } },
      { "type": "branch", "size": 560, "data": [
        { "type": "leaf", "size": 400, "data": { "id": "group-2", "views": ["viewport"], "activeView": "viewport" } },
        { "type": "leaf", "size": 160, "data": { "id": "group-3", "views": ["perf"], "activeView": "perf" } }
      ]}
    ]
  },
  "width": 800,
  "height": 600,
  "orientation": "HORIZONTAL"
}
```

### Layer 3 — DockComponent (tabs + DnD + orchestration)

Each leaf in the grid tree is a **group** — a tabbed container holding 1+ panels.

**Tab management:**
- Each group has a `panels: Panel[]` array and an `activePanel: Panel`
- MRU (most recently used) list — closing the active tab activates the MRU head, not the adjacent tab
- Tabs can be dragged between groups

**Drop zones:**
- 5 zones per group: center (tabify), top/right/bottom/left (split)
- Computed as 20% edge threshold of the target element
- "Center" adds the panel as a tab in the existing group
- Edge drops create a new group at the grid location

**Panel lifecycle:**
- `movingLock()` suppresses intermediate events during multi-step operations (prevents a group from auto-closing when temporarily empty mid-move)
- Panel state is preserved per-group (Blender's SpaceLink pattern)

---

## How Blender Differs

Blender uses a vertex-edge-area graph instead of a tree. The key differences:

| Feature | dockview (tree) | Blender (graph) |
|---|---|---|
| T-junctions | Implicit via tree nesting | Explicit via shared vertices |
| Resize propagation | Per-splitview, no cross-axis | All collinear edges move together |
| Merge | Collapse single-child branches | Remove shared edge, reassign vertices |
| Constraint solver | Greedy linear walk | Iterative multi-pass (up to 10) |
| Coordinates | Pixel integers | Short integers (absolute) |
| Cleanup | Not needed (tree is always valid) | Required after every mutation |

**Blender's advantages:** More flexible layouts (arbitrary T-shapes), simpler merge semantics.
**dockview's advantages:** Simpler implementation, no cleanup pipeline, proven in web browsers, DOM-native.

**For Phi:** Use the dockview/tree architecture. It's simpler, proven on the web, and the tree restructuring handles T-junctions automatically. Blender's graph model is elegant but its complexity is justified by C/GPU rendering, not browser DOM.

---

## Implementation Plan

### Phase 1 — Splitview

Build the 1D constraint solver as a Svelte component.

- `Splitview.svelte` — renders N children separated by sash handles
- **DOM-based sizing** — `element.style.flexBasis = size + 'px'` in a flex container, OR absolute `width`/`height`
- Greedy constraint solver: `resize(sashIndex, delta, snapshot)`
- `distributeEmptySpace()` invariant enforcer
- Proportional resize via saved snapshot + `ResizeObserver`
- Sash drag: `setPointerCapture`, snapshot on pointerdown, delta from start

### Phase 2 — Gridview

Build the recursive tree that nests Splitviews.

- `Gridview` class — manages the `BranchNode`/`LeafNode` tree
- `addView(location, view)` — insert with tree restructuring
- `removeView(location)` — remove with branch collapse
- `moveView(from, to)` — combined remove + add
- Serialization: `toJSON()` / `fromJSON()`

### Phase 3 — DockLayout

The Svelte component that adds tabs, DnD, and panel management.

- `DockLayout.svelte` — wraps Gridview, renders groups as tabbed containers
- `DockGroup.svelte` — tab bar + content area
- Drop zone overlays (5-zone)
- Panel registry + snippet-based rendering
- Workspace save/load (localStorage)

### Phase 4 — Integration

Replace Gestalt's current Sidebar/PanelArea with DockLayout.

---

## What to Delete

Our current `packages/phi/src/dock/` implementation should be replaced entirely:
- `types.ts` — vertex-edge-area types (replace with tree types)
- `graph.ts` — graph operations (replace with tree operations)
- `DockLayout.svelte` — CSS Grid renderer (replace with DOM-based Splitview nesting)
- `DockArea.svelte` — keep the concept, rewrite internals
- `DockTabs.svelte` — keep as-is, it's fine
- Tests — rewrite for new architecture

---

## Key Constants (from dockview + Blender)

| Constant | Value | Source |
|---|---|---|
| Minimum panel width | 180px | Our choice (Blender: 29px unscaled) |
| Minimum panel height | 100px | Our choice (Blender: 26px header) |
| Sash size | 4px | dockview default |
| Drop zone edge threshold | 20% | dockview default |
| Snap threshold | 50% of view minimum | dockview |
| Proportional precision | Save as `size / totalSize` floats | dockview |

---

## DnD Implementation Details (from dockview source, 2026-03-27)

### Drop Zone Overlay Rendering

Dockview renders drop zone overlays as **DOM elements**, not canvas or SVG.

When a drag enters a target group:
1. A `<div class="dv-drop-target-dropzone">` is created and appended to the target element
2. Inside it, a `<div class="dv-drop-target-selection">` serves as the visual highlight
3. Position is set via CSS `top/left/width/height` — not CSS transforms for positioning
4. GPU compositing is enabled via `translate3d(0, 0, 0)` and `will-change: transform` on the overlay
5. Transition timing: 70ms ease for position/size changes (smooth zone-to-zone transitions during drag)
6. CSS classes toggle per zone: `dv-drop-target-left`, `dv-drop-target-right`, `dv-drop-target-top`, `dv-drop-target-bottom`, `dv-drop-target-center`

The overlay size is configurable via an `overlayModel` with `activationSize` (how close to edge before zone activates) and `size` (how large the overlay renders). Both can be percentage-based or pixel-based.

For complex layouts, `DropTargetAnchorContainer` renders overlays positioned absolutely within a parent container, allowing the overlay to extend beyond a single target's bounds.

**For Phi:** Use the same DOM overlay approach. Create the overlay `<div>` on drag enter, position it with CSS, remove on drag leave. Use `will-change: transform` for GPU compositing. 70ms transition for smooth zone switching.

### Tab Drag Initiation

Dockview uses **native HTML5 drag-and-drop**. There is no custom pixel-distance threshold.

- Each tab element has `draggable="true"` set as an HTML attribute
- When the browser fires `dragstart` (typically after 4-5px of movement, browser-dependent), the `DragHandler` sets `dataTransfer` data with a `PanelTransfer` payload
- A ghost image is configured via `dataTransfer.setDragImage()`
- There is no custom `pointerdown` + `pointermove` distance check — the browser handles click vs drag distinction

**For Phi:** Set `draggable="true"` on tab elements in DockTabs.svelte. Listen for `dragstart` to set payload, `dragend` for cleanup. Let the browser handle the click/drag threshold. Use `dataTransfer.setData("application/json", JSON.stringify({ panelId, groupId }))` for the payload.

### Cross-Group Panel Transfer (Without Destroying Components)

Dockview moves panels between groups via `moveGroupWithoutDestroying()`:

1. **Remove from source:** `sourceGroup.model.removePanel(panel)` detaches the panel from the group's tab list and render container
2. **Detach rendering:** `renderContainer.detatch(panel)` removes the panel's DOM from the source group's rendering context without destroying the component instance
3. **Re-attach to destination:** `destinationGroup.model.openPanel(panel)` adds the panel to the new group's tab list and render container
4. **DOM reuse:** The panel's underlying DOM element is moved (reparented), not recreated. Component state persists because the instance was never disposed.

If the source group becomes empty after the panel leaves, it is removed from the grid tree (triggering the automatic branch collapse from `removeView()`).

**For Phi:** The key is a render container system that separates panel lifecycle from DOM attachment. When a panel moves groups:
- Remove it from the source DockGroup's panel list
- Move the DOM node to the destination DockGroup's content area (`destination.contentEl.appendChild(panel.element)`)
- Add it to the destination's panel list
- Since Svelte 5 components are mounted to a target element, reparenting the mount target preserves the component instance

**Important:** This requires that panel content is rendered into a persistent DOM element (not conditionally rendered via `{#if}` blocks that would destroy/recreate on move). Each panel should have a dedicated container element that exists for the panel's lifetime, independent of which group currently hosts it.

### Serialization: Pixel Sizes with Proportional Restoration

Dockview stores **absolute pixel sizes** in serialization, not fractions.

Serialized format:
```typescript
interface SerializedGridObject<T> {
  type: "leaf" | "branch";
  data: T | SerializedGridObject<T>[];
  size?: number;    // pixels at time of save
  visible?: boolean;
}
```

On deserialization:
1. The saved tree structure is reconstructed with the original pixel sizes
2. `grid.layout(savedWidth, savedHeight)` is called
3. If `proportionalLayout: true` (the default), the Splitview distributes space proportionally based on the ratio of saved sizes
4. The grid is then re-laid-out to the **current** container dimensions, which scales proportionally

Example: A grid saved as 300px / 700px in a 1000px container. Restored on a 2000px screen → becomes 600px / 1400px automatically. The Splitview's proportional layout mode handles this.

**For Phi:** Our `Gridview.serialize()` already stores pixel sizes. The proportional resize logic in Splitview already handles different container sizes. The missing piece is the localStorage plumbing and view factory:
```typescript
// Save
localStorage.setItem("workspace", JSON.stringify(gridview.serialize()));

// Restore
const saved = JSON.parse(localStorage.getItem("workspace"));
if (saved) {
  const gv = Gridview.deserialize(saved, (id) => createPanelView(id));
}
```

The view factory (`createPanelView(id)`) maps panel IDs to IGridView instances. In App.svelte, this is currently `createPanelView()` — already exists but only used for initial layout. Reuse it for deserialization.

---

## Current Implementation Status (updated 2026-03-27)

| Feature | Status | What Exists | What's Missing |
|---|---|---|---|
| **Splitview** | Complete | Constraint solver, sash drag, proportional resize, 44 tests | — |
| **Gridview** | Complete | Tree add/remove/restructure, collapse, serialize/deserialize, 38 tests | — |
| **DockLayout** | Complete | Absolute positioning, sash drag, group rendering | — |
| **DockTabs** | Complete | Tab bar, close buttons, active tab switching | DnD drag initiation |
| **DockGroup** | Complete | Tabbed container, content rendering | Drop zone overlays |
| **DnD** | Not started | — | Tab `draggable`, drop zone overlays, 5-zone detection, panel transfer |
| **Merge** | Partial | Tree collapse in removeView() works | User gesture (drag outward, close last tab) |
| **Persistence** | Partial | serialize/deserialize tested, types exported | localStorage wiring, view factory in App.svelte |
