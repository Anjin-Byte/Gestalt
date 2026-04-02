# Layout Rendering: CSS Grid vs Absolute Positioning

**Type:** reference
**Status:** current
**Date:** 2026-03-21
**Context:** Given that the vertex-edge-area graph is isomorphic to a CSS Grid definition, should we render via Grid or absolute positioning?

---

## The Isomorphism

A valid rectilinear partition (vertex-edge-area graph) maps 1:1 to a CSS Grid:

```
Vertices:  (0,0) (300,0) (500,0) (0,200) (300,200) (500,200) (0,400) (500,400)

Unique X: [0, 300, 500]  →  grid-template-columns: 300px 200px
Unique Y: [0, 200, 400]  →  grid-template-rows: 200px 200px

Area A (0,0)→(300,400):   grid-column: 1/2; grid-row: 1/3  (spans 2 rows)
Area B (300,0)→(500,200): grid-column: 2/3; grid-row: 1/2
Area C (300,200)→(500,400): grid-column: 2/3; grid-row: 2/3
```

This is an L-shaped layout (T-junction). CSS Grid handles it natively.

---

## Operation-by-Operation Comparison

### 1. Edge Drag (Resize)

**Absolute positioning:**
```typescript
// Move vertices
vertex.x += delta;
// Recompute affected areas
for (const area of affectedAreas) {
  el.style.left = area.v1.x + 'px';
  el.style.width = (area.v3.x - area.v1.x) + 'px';
}
```
One style mutation per affected area. Manual clamping for min sizes.

**CSS Grid:**
```typescript
// Move vertices in the graph
vertex.x += delta;
// Recompute grid template from all unique X coordinates
const xs = getUniqueX(screen);
container.style.gridTemplateColumns = xs.map((x, i) =>
  i === 0 ? '' : `${xs[i] - xs[i-1]}px`
).join(' ');
```
One style mutation total (the grid template). Min sizes enforced by `min-width` on area elements — the browser clamps track sizes automatically.

**Winner: CSS Grid.** Fewer DOM mutations. Browser handles constraint propagation.

### 2. Window Resize (Proportional Scale)

**Absolute positioning:**
Must run the iterative constraint solver (up to 10 passes), recompute all vertex positions, update all area styles.

**CSS Grid with `fr` units:**
```css
grid-template-columns: 3fr 2fr;  /* ratio 0.6 : 0.4 */
grid-template-rows: 2fr 2fr;
```
The browser scales proportionally automatically. Min sizes respected via `minmax(MIN, Xfr)`. No JavaScript needed for the common case.

**Winner: CSS Grid.** The browser's grid layout algorithm IS the proportional scaler with constraint solving built in.

### 3. Split Operation

**Absolute positioning:**
Create new vertices and edges in the graph, compute new area positions, create new DOM element with absolute coordinates.

**CSS Grid:**
Create new vertices and edges in the graph, recompute grid template (adds a new track), create new DOM element with `grid-column`/`grid-row` spans. The browser handles sizing.

**Winner: Tie.** Both require the same graph mutation. Rendering cost is similar.

### 4. Merge Operation

**Absolute positioning:**
Remove consumed area, update surviving area's vertices, recompute position/size, remove DOM element.

**CSS Grid:**
Remove consumed area, update surviving area's vertices, recompute grid template (may remove a track), update surviving element's `grid-column`/`grid-row` spans, remove DOM element.

**Winner: Tie.** Same graph mutation. Grid needs span update, absolute needs position update.

### 5. Splitter Handle Rendering

**Absolute positioning:**
Must create and position splitter elements manually along each internal edge.

**CSS Grid:**
Use `gap` property for the splitter gutter. Splitter handles are naturally positioned between tracks. OR: use a dedicated track of fixed size (e.g., `4px`) for each splitter.

**Winner: CSS Grid.** `gap` or fixed-size tracks handle splitter positioning automatically.

### 6. Animation (Split/Merge Transitions)

**Absolute positioning:**
Interpolate vertex positions with `requestAnimationFrame`. Full control over timing.

**CSS Grid:**
CSS `transition` on `grid-template-columns` / `grid-template-rows` is not well-supported across browsers. Would need to animate via JavaScript anyway, or use absolute positioning during animation and snap to grid after.

**Winner: Absolute positioning** for animation. But this is a progressive enhancement — grid works fine without animation.

### 7. Minimum Size Enforcement

**Absolute positioning:**
Manual clamping in the constraint solver. Must propagate through the graph.

**CSS Grid:**
```css
.area { min-width: 180px; min-height: 100px; }
```
Or per-track: `grid-template-columns: minmax(180px, 3fr) minmax(180px, 2fr);`
The browser's grid algorithm enforces minimums automatically, including for spanning areas.

**Winner: CSS Grid.** The browser does the constraint math for free.

### 8. Subpixel Rendering

**Absolute positioning:**
Integer pixels from vertex coordinates. Rounding can cause 1px gaps between adjacent areas.

**CSS Grid:**
The browser handles subpixel distribution. No gaps. Adjacent areas share exact pixel boundaries because they share grid lines.

**Winner: CSS Grid.** No 1px gap bugs.

---

## Summary

| Operation | Absolute | CSS Grid | Notes |
|---|---|---|---|
| Edge drag | Good | **Better** | Fewer mutations, browser clamps minimums |
| Window resize | Manual solver | **Browser-native** | `fr` units + `minmax()` |
| Split | Tie | Tie | Same graph mutation |
| Merge | Tie | Tie | Same graph mutation |
| Splitter handles | Manual | **`gap` / tracks** | |
| Animation | **Better** | Weak | Grid template transitions poorly supported |
| Min sizes | Manual solver | **Browser-native** | `minmax()` per track |
| Subpixel | 1px gap risk | **No gaps** | Shared grid lines |

**Score: CSS Grid 5, Absolute 1, Tie 2.**

---

## Proposed Architecture: Graph Model + CSS Grid Renderer

Keep the vertex-edge-area graph as the **data model** for split/merge/validation logic. Render via CSS Grid. The conversion algorithm runs on every layout change:

```typescript
function graphToGrid(screen: Screen): GridDefinition {
  const xs = [...new Set(screen.vertices.map(v => v.x))].sort((a, b) => a - b);
  const ys = [...new Set(screen.vertices.map(v => v.y))].sort((a, b) => a - b);

  // Convert pixel gaps to fr ratios for proportional scaling
  const totalW = xs[xs.length - 1] - xs[0];
  const totalH = ys[ys.length - 1] - ys[0];

  const columns = xs.slice(1).map((x, i) => {
    const size = x - xs[i];
    return `minmax(${MIN_AREA_WIDTH}px, ${size / totalW}fr)`;
  });

  const rows = ys.slice(1).map((y, i) => {
    const size = y - ys[i];
    return `minmax(${MIN_AREA_HEIGHT}px, ${size / totalH}fr)`;
  });

  const placements = screen.areas.map(area => ({
    id: area.id,
    gridColumn: `${xs.indexOf(area.v1.x) + 1} / ${xs.indexOf(area.v3.x) + 1}`,
    gridRow: `${ys.indexOf(area.v1.y) + 1} / ${ys.indexOf(area.v3.y) + 1}`,
  }));

  return {
    gridTemplateColumns: columns.join(' '),
    gridTemplateRows: rows.join(' '),
    placements,
  };
}
```

### Edge Drag with Grid

During edge drag:
1. Update vertex coordinates in the graph.
2. Run `graphToGrid()` to get new track sizes.
3. Set `gridTemplateColumns` / `gridTemplateRows` — one DOM mutation.
4. The browser handles everything else (constraint propagation, subpixel rounding, min sizes).

### Splitter Tracks

Option A: CSS `gap` + invisible overlay handles.
Option B: Dedicated 4px tracks between content tracks, explicitly styled as splitter elements.

Option B is more controllable and matches Blender's approach where splitter edges are real geometric entities.

---

## What Changes from the Research Document

The vertex-edge-area graph data model stays the same. The cleanup pipeline stays the same. Split/merge algorithms stay the same. What changes:

1. **Rendering**: CSS Grid instead of absolute positioning.
2. **Window resize**: `fr` units handle proportional scaling natively. The iterative solver becomes a fallback for extreme cases only.
3. **Min size enforcement**: `minmax()` tracks instead of manual clamping.
4. **Vertex coordinates**: Store as fractions (0–1) rather than pixels. Convert to `fr` ratios for the grid template.
5. **No 1px gap bugs**: Grid lines are shared by definition.

The graph model is the **source of truth for topology** (what's connected to what). CSS Grid is the **layout engine** (how big things are and where they go). Clean separation.
