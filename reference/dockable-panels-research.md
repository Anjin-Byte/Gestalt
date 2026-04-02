# Dockable Panel Systems — Research Summary

**Type:** reference
**Status:** current
**Date:** 2026-03-21
**Scope:** Architecture research for implementing a vertex-edge-area dockable panel system in Gestalt (Svelte/TypeScript), modeled on Blender's screen layout system.

---

## Architecture Decision: Vertex-Edge-Area Graph

Gestalt will use Blender's **vertex-edge-area graph** model rather than a binary split tree. This is the most powerful layout representation — it handles T-shapes, L-shapes, and arbitrary merges naturally. No web-based docking library currently uses this approach.

---

## Data Model

The layout is a planar graph of shared vertices, edges, and rectangular areas:

### Core Structures

**ScrVert** — a 2D corner point. Integer coordinates (eliminates floating-point alignment bugs). Shared between adjacent areas — moving a vertex resizes all areas that reference it.

```typescript
interface ScrVert {
  id: string;
  x: number;  // integer pixels
  y: number;  // integer pixels
}
```

**ScrEdge** — a border between areas. Always horizontal or vertical. Connects two vertices.

```typescript
interface ScrEdge {
  v1: ScrVert;  // sorted: v1 pointer < v2 pointer
  v2: ScrVert;
  border: boolean;  // true = window boundary edge
}
```

**ScrArea** — a rectangular editor panel. Defined by four vertex references (bottom-left, top-left, top-right, bottom-right).

```typescript
interface ScrArea {
  id: string;
  v1: ScrVert;  // bottom-left
  v2: ScrVert;  // top-left
  v3: ScrVert;  // top-right
  v4: ScrVert;  // bottom-right
  panelStates: Map<string, unknown>;  // editor state per panel type (SpaceLink pattern)
  activePanel: string;
}
```

**Screen** — the top-level container.

```typescript
interface Screen {
  vertices: ScrVert[];
  edges: ScrEdge[];
  areas: ScrArea[];
}
```

### Invariants

1. **Complete tiling** — every pixel belongs to exactly one area. No gaps.
2. **No overlaps** — area rectangles never overlap.
3. **Vertex sharing** — adjacent areas share vertices at common corners.
4. **Edge completeness** — every side of every area has corresponding edges.
5. **Axis alignment** — all edges are horizontal or vertical.
6. **Integer coordinates** — exact comparison, no epsilon needed.

---

## Split Operation

Split divides one area into two along a horizontal or vertical axis.

### Horizontal Split Algorithm

Given area with vertices v1(BL), v2(TL), v3(TR), v4(BR) and factor `fac` (0.0–1.0):

1. Compute split Y coordinate, clamped to minimum area heights.
2. Create two new vertices: `sv1 = (v1.x, splitY)`, `sv2 = (v4.x, splitY)`.
3. Create five new edges: left-bottom, left-top, right-top, right-bottom, and the split edge itself.
4. Assign vertices: if `fac > 0.5`, new area gets the top half (`sv1, v2, v3, sv2`), original shrinks to bottom (`v1, sv1, sv2, v4`). Otherwise reversed.
5. Copy editor state into the new area.
6. Run cleanup pipeline.

### Vertical Split

Same pattern, rotated 90°. New vertices at `(splitX, v1.y)` and `(splitX, v2.y)`.

### Post-Split Cleanup

```
removeDoubleVertices()  → merge vertices at identical coordinates
removeDoubleEdges()     → remove duplicate edges
removeUnusedEdges()     → remove edges not referenced by any area
removeUnusedVertices()  → remove vertices not referenced by any edge
```

This is a **garbage collection** approach: mutate aggressively, then clean up. More robust than trying to maintain invariants during mutations.

---

## Merge (Join) Operation

The hardest operation. Three tiers of complexity.

### Tier 1: Aligned Join

Two areas share an entire edge (borders align perfectly). Reassign vertex pointers so the surviving area absorbs the other. Delete the consumed area.

**Orientation detection**: check if areas share a horizontal or vertical edge with sufficient overlap (tolerance = `AREAJOINTOLERANCEX/Y`).

### Tier 2: Unaligned Join

Areas are adjacent but don't align perfectly (one is taller/wider than the other).

1. Compute misalignment offsets at each end.
2. **Trim remainders**: split off the non-overlapping portion of the larger area into a temporary "remainder" area.
3. Join the now-aligned areas (Tier 1).
4. Close remainders by auto-merging them with their best-aligned neighbor.

### Tier 3: Auto-Close

When a remainder area needs elimination, find the best neighbor to absorb it. Score neighbors by how well their shared edge length matches. Join with the best match.

### Pre-Join Vertex Alignment

Before joining, average vertices to eliminate small misalignments:
```
top = (sa1.v2.y + sa2.v2.y) / 2
// Move ALL vertices at the old coordinates to the averaged coordinate
```
This is a **global operation** affecting all areas sharing those vertex coordinates.

---

## Constraint Solving During Resize

### Edge Drag

1. Hit-test to find the dragged edge (2px+ margin).
2. Select ALL collinear edges along the same axis — dragging one edge moves the entire row/column.
3. Compute movement limits: for each adjacent area, `limit = area.dimension - MIN_SIZE`.
4. Clamp delta to the minimum of all limits on each side.
5. Move all flagged vertices to the new position.

**No chain propagation** — movement is hard-clamped. The edge stops when any adjacent area hits its minimum.

### Window Resize (Proportional Scaling)

Iterative constraint solver, up to 10 passes:

1. Compute scale factors: `scaleX = newWidth / oldWidth`, `scaleY = newHeight / oldHeight`.
2. Scale all vertex positions proportionally.
3. For each area below minimum size, push vertices apart. If any were adjusted, repeat.
4. After 10 passes, give up — some areas may be below minimum (flagged as too-small, regions hidden).

Use `ResizeObserver` on the container element, not `window.onresize`.

---

## CSS Rendering Strategy

### Absolute Positioning (Recommended)

Each area is `position: absolute` with `left/top/width/height` computed directly from vertex coordinates. This exactly mirrors Blender's approach.

```svelte
<div class="screen" style="position: relative; width: 100%; height: 100%;">
  {#each areas as area}
    <div class="area" style="
      position: absolute;
      left: {area.v1.x}px;
      top: {containerHeight - area.v3.y}px;
      width: {area.v3.x - area.v1.x}px;
      height: {area.v3.y - area.v1.y}px;
    ">
      <!-- panel content -->
    </div>
  {/each}
</div>
```

Note: Y-axis inverted (Blender Y goes up, CSS Y goes down).

### Why Not CSS Grid?

CSS Grid **can** represent arbitrary rectilinear partitions (extract unique coordinates → grid lines, each area spans correct cells). But:

- During edge drag, you'd need to recompute all track sizes. Absolute positioning only updates a few vertex coordinates.
- The vertex-edge-area graph IS the layout. Duplicating into CSS Grid creates synchronization burden.
- Split/merge animations are trivial with absolute positioning — just interpolate vertex positions.

### CSS Grid Can Be Used Within Areas

Each area's internal layout (header, toolbar, main content, sidebar) can use CSS Grid or flexbox. The absolute positioning only applies to the area containers themselves.

---

## Action Zones (Interaction Hotspots)

### Corner Zones (Split / Merge)

Each area has four corner action zones (~20×20px). The gesture is determined by drag direction:

- **Drag inward** (into the area): **Split**. Drag direction determines split axis.
- **Drag outward** (into a neighboring area): **Merge**. The neighbor is consumed.

### Gesture Detection

1. Track delta from initial click point.
2. Dead zone: `< 0.1 * widgetUnit` (~2px) — no action.
3. Direction: 45° sectors (N/S/E/W).
4. Merge threshold: `>= 0.6 * widgetUnit` (~12px) if direction leaves the area.
5. Split threshold: `>= 1.2 * widgetUnit` (~24px) if direction stays within.

### Edge Zones (Resize)

Invisible hit zones along area borders. Cursor changes to `ns-resize` or `ew-resize`. Splitter drag uses `setPointerCapture` for smooth tracking (same pattern as PanelArea's existing resize handle).

---

## Editor State Preservation (SpaceLink Pattern)

Each area keeps a `Map<PanelType, SerializedState>` — a record of all panel states that have ever occupied that slot.

### Swap Algorithm

1. Serialize current panel's state into the map.
2. Check if the target panel type has a previous state in the map.
   - If yes: restore it.
   - If no: create a fresh default state.
3. In Svelte: use `{#if}` or `{#key}` to swap the component, passing the restored state as props.

This means switching from Viewport to Debug and back restores full state (camera angle, scroll position, selection, etc.).

---

## Notifier System (Selective Redraw)

Panels subscribe to typed data change events. Only panels that care about a specific change redraw.

### Blender's Pattern

1. Data changes post notifiers: `NC_SCENE | ND_OB_ACTIVE` (scene category, active object changed).
2. Notifiers queue and deduplicate.
3. Each area/region's `listener` decides whether to tag for redraw.
4. Only tagged regions redraw.

Philosophy: notifiers describe **what happened**, not **what should happen**.

### Svelte Mapping

- Scoped Svelte stores per data domain (scene store, GPU pool store, performance store).
- Each panel subscribes to relevant stores via `$derived`.
- Svelte's reactivity handles the "only update what changed" automatically.
- Hidden tabs (inactive in a TabNode) must skip rendering — check visibility before expensive work.

---

## Serialization

### What to Save

```typescript
interface WorkspaceLayout {
  vertices: { id: string; x: number; y: number }[];
  edges: { v1: string; v2: string; border: boolean }[];
  areas: {
    id: string;
    v1: string; v2: string; v3: string; v4: string;  // vertex IDs
    activePanel: string;
    panelStates: Record<string, unknown>;
  }[];
}
```

Positions stored as **fractions (0–1)** of the container dimensions for resolution independence. Convert to integer pixels on load.

### Workspace Presets

Named layouts that can be saved, loaded, and reset to defaults. Stored in `localStorage` keyed by preset name.

---

## Validation and Recovery

### Cleanup Pipeline (Run After Every Mutation)

```typescript
function cleanup(screen: Screen): void {
  removeDoubleVertices(screen);  // merge vertices at identical coordinates
  removeDoubleEdges(screen);     // remove duplicate edges
  removeUnusedEdges(screen);     // remove edges not referenced by areas
  removeUnusedVertices(screen);  // remove vertices not referenced by edges
}
```

### Validation Function

Check all invariants: 4 distinct vertices per area, proper rectangle geometry, total area equals container area, no overlaps. If validation fails, fall back to a single full-screen area.

### `removeDoubleVertices()` — Exact Algorithm

1. For each vertex, scan all subsequent vertices for identical coordinates.
2. Mark duplicates with a `redirect` pointer to the canonical vertex.
3. Redirect all edge and area references from duplicate → canonical.
4. Delete marked vertices.

Integer comparison — no epsilon needed.

---

## Key Constants

| Constant | Value | Purpose |
|---|---|---|
| `AREA_MIN_X` | 29 | Minimum area width (unscaled pixels) |
| `HEADER_HEIGHT` | 26 | Header height (20 + 6 padding) |
| `JOIN_TOLERANCE_X` | `AREA_MIN_X * scale` | Horizontal merge tolerance |
| `JOIN_TOLERANCE_Y` | `HEADER_HEIGHT * scale` | Vertical merge tolerance |
| `BORDER_PADDING` | ~5 | Edge click detection margin |
| `GESTURE_DEAD_ZONE` | ~2 | Minimum drag before gesture activates |
| `MERGE_THRESHOLD` | ~12 | Drag distance to trigger merge |
| `SPLIT_THRESHOLD` | ~24 | Drag distance to trigger split |
| `MAX_SCALE_PASSES` | 10 | Iterative constraint solver limit |

---

## Implementation Order

1. **Data model + invariant validation** — `ScrVert`, `ScrEdge`, `ScrArea`, `Screen` types. Cleanup pipeline. Validation function. Tests for all invariants.
2. **Absolute positioning renderer** — Svelte component that renders areas from vertex coordinates. `ResizeObserver` + proportional scale.
3. **Edge resize** — hit-test edges, `setPointerCapture` drag, vertex position update with min-size clamping.
4. **Split operation** — corner action zones, gesture detection, split algorithm, cleanup.
5. **Merge operation** — aligned join, unaligned join with trim, auto-close.
6. **Tab container** — tab bar within each area, panel switching, editor state preservation (SpaceLink pattern).
7. **Panel registry** — register panel types with components and default config.
8. **Drag-and-drop panel rearrangement** — drop zone overlays for moving panels between areas.
9. **Serialization** — workspace save/load/reset, named presets.
10. **Workspace presets** — default layouts, user-defined layouts, preset switcher.

---

## Sources

- [Blender Developer Docs — Screen](https://developer.blender.org/docs/features/interface/screen/)
- [DeepWiki — Screen and Area Management](https://deepwiki.com/blender/blender/3.3-screen-and-area-management)
- [DNA_screen_types.h](https://projects.blender.org/blender/blender/raw/branch/main/source/blender/makesdna/DNA_screen_types.h)
- [screen_edit.cc](https://projects.blender.org/blender/blender/raw/branch/main/source/blender/editors/screen/screen_edit.cc)
- [screen_geometry.cc](https://projects.blender.org/blender/blender/raw/branch/main/source/blender/editors/screen/screen_geometry.cc)
- [screen_ops.cc](https://projects.blender.org/blender/blender/raw/branch/main/source/blender/editors/screen/screen_ops.cc)
- [Blender Notifier Architecture](https://archive.blender.org/wiki/2015/index.php/Dev:2.5/Source/Architecture/Window_Manager/)
- [Blender Areas Manual](https://docs.blender.org/manual/en/latest/interface/window_system/areas.html)
- [Lumino (JupyterLab layout)](https://github.com/jupyterlab/lumino)
- [dockview](https://github.com/mathuo/dockview)
- [Rectangular Partitions (arXiv)](https://arxiv.org/pdf/2111.01970)
