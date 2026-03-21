# Outliner Pattern — Research Document

**Type:** reference
**Status:** current
**Date:** 2026-03-21
**Scope:** The Blender Outliner as a UI pattern for per-object state inspection across all domains of a 3D tool — not just GPU debugging.

---

## Reference: The Blender Outliner

The Blender Outliner is the canonical implementation of a **dense, scannable, per-object state panel**. Its defining properties:

- **Uniform row height** — every object occupies the same vertical space; the list becomes a grid you can scan at a fixed rhythm
- **Icon + name** — type identity before you read the label; you know what kind of thing it is before processing the name
- **Inline state columns** — visibility, viewport display, render are togglable directly in the row without opening any sub-panel. State is visible and mutable in the same gesture
- **Row-level color** — grayed rows are hidden; orange rows are selected; the entire list doubles as a status board
- **Collapse / expand** — hierarchy without losing sibling context
- **Filter bar** — collapses the list to matching rows; essential at scale

The Outliner is effective as both a *scene management* tool and a *debugger* panel because it makes **state scannable across all objects simultaneously** rather than forcing you to select each one to inspect it.

---

## The Pattern's Core Principle

> **A row is not just a name. A row is a status report.**

Each row is a compressed summary of an object's full state. You read left-to-right (type → name → state columns) and top-to-bottom (hierarchy order). Anomalies jump out without requiring selection or drill-down.

---

## Scope in a 3D Tool

The Outliner is not a single component — it is a **structural pattern** that recurs across every major concern of a 3D tool. Each domain has different row data but the same underlying structure: a hierarchical list where each row carries its own inline state.

### Domain Map

| Domain | What the rows are | Key inline state |
|---|---|---|
| **Scene** | Objects, meshes, lights, cameras | Visibility, render, selection |
| **Materials** | Materials, textures, shader inputs | Slot assignment, bind status |
| **Animation** | Tracks, keyframes, constraints | Active, muted, influence weight |
| **GPU Resources** | Buffer slots, pools, pages | Occupancy, dirty, version |
| **Render Pipeline** | Passes, sub-passes, barriers | Duration, budget status, overlap |
| **World / Chunks** | Spatial chunks, LOD levels | Built, culled, version mismatch |
| **Lighting** | Cascade probes, light sources | Probe state, ray hit count |

In Blender, separate Outliner modes (Object Mode, Data Blocks, Scenes, etc.) switch the domain being shown — same component, different data. That is the right model: one `OutlinerRow` / `OutlinerGroup` primitive, many consumers.

---

## Application to Gestalt — Current Panels

### 1. GpuPoolPanel — Buffer Pool Outliner

**Current state:** Flat `BarMeter` rows showing aggregate occupancy only.

**Outliner form:** Each row = one allocated buffer slot.

| Column | Content |
|---|---|
| Type icon | Chunk mesh, summary, cascade, etc. |
| Name / ID | Chunk coordinate or slot index |
| Status dot | Clean / dirty / rebuild-pending / eviction-candidate |
| Version | Current version number |
| Size | Memory footprint |
| Age | Frames since last use (eviction pressure) |

`BarMeter` is promoted to group header aggregate — not replaced.

---

### 2. ScenePanel — Scene Object Outliner

**Current state:** Unknown — needs audit. Likely a flat property list.

**Outliner form:** Each row = one scene object (mesh, light, camera).

| Column | Content |
|---|---|
| Type icon | Mesh, point light, directional, camera |
| Name | Object name |
| Visibility | Eye icon toggle (viewport) |
| Render | Camera icon toggle (included in render) |
| Selection | Highlighted row = selected object |

This is the most "traditional" outliner use — scene composition and selection. It is what users coming from Blender, Maya, or any 3D DCC expect to exist in a tool like this.

---

### 3. PerformancePanel — Render Pass Outliner

**Current state:** `TimelineCanvas` shows pass durations but no structural relationships.

**Outliner form:** Each row = one render pass, with child rows for dependent resources.

| Column | Content |
|---|---|
| Pass icon | Compute / render / blit / barrier |
| Pass name | R-2 Depth Prepass, R-4a Chunk Cull, etc. |
| Duration sparkline | Per-frame history |
| Last ms | Current frame cost |
| Budget dot | Green / amber / red vs budget |

Child rows of a pass could show its input/output resources (the depth buffer produced by R-2 that is consumed by R-3, etc.). This turns the pass list into a dependency graph made linear.

---

### 4. DebugPanel — Chunk Visibility Outliner

**Current state:** Aggregate counters only. No per-chunk visibility.

**Outliner form:** Each row = one active chunk.

| Column | Content |
|---|---|
| Chunk coord | (x, y, z) in grid space |
| Visibility | Visible / frustum-culled / Hi-Z-occluded / empty |
| Version | CPU vs GPU-side version |
| Slots | Buffer slot count |
| Rebuild | Dirty flag |

Highest GPU-side cost — requires per-chunk readback. The component is straightforward; the readback path is the blocker.

---

### 5. Future: Lighting / Cascade Outliner

Radiance cascade probes as rows. Each probe shows:
- Probe index and world position
- Ray hit count (sparkline)
- Merge status
- Cascade level it belongs to (indent depth = cascade level)

---

## Shared Component Architecture

All domains share the same DOM primitives. The data wired into them changes per domain.

### `OutlinerRow`

```
[ indent ] [ icon ] [ name ········· ] [ col1 ] [ col2 ] [ col3 ]
```

- CSS grid columns — fixed widths, aligned across all rows in a list
- `depth` prop drives left indent (multiples of 12px)
- `selected` state → `--interactive-fill` background
- `expandable` prop → chevron in indent area
- `onclick`, `onexpand` callbacks
- Named column slots via Svelte snippets

### `OutlinerGroup`

A collapsible group header row sitting above child `OutlinerRow`s.

```
[ ▶ ] [ group name ] [ aggregate / BarMeter ] [ count ]
```

- Collapse state persisted to `localStorage` (same pattern as `Section`)
- Group header is also a row — same height, same grid, same hover

### `StatusCell`

A fixed-width cell showing a status dot and optional short label.

```
[ ● clean ]   [ ● dirty ]   [ ● rebuilding ]   [ — ]
```

- `--color-success` / `--color-warning` / `--color-destructive` / `--text-faint`
- Used in any column slot of `OutlinerRow`

### `InlineSparkCell`

A fixed-width cell wrapping `Sparkline` at 48–64px.

- Extracted from the existing `CounterRow` layout
- Accepts `values`, `warn`, `danger` passthrough

---

## Layout Constraints

The panel is 180–520px wide. Column budget at default 280px:

| Column | Width |
|---|---|
| Indent + icon + name | flex: 1 (~120px min) |
| Status dot | 16px |
| Value (mono) | 48px |
| Sparkline | 52px |
| Secondary value | 36px |

**Adaptive column visibility:** At narrow widths (< 300px), secondary columns (sparkline, age) are hidden. The name column always gets remaining space. CSS container queries are the right mechanism — no JS needed.

---

## Relationship to Existing Components

The Outliner does not replace existing components — it sits above them in the hierarchy:

| Existing | Role after Outliner |
|---|---|
| `PropRow` | Still used for scalar key/value pairs |
| `BarMeter` | Promoted to `OutlinerGroup` aggregate header |
| `CounterRow` / `Sparkline` | Absorbed into `InlineSparkCell` |
| `StatusIndicator` | Absorbed into `StatusCell` |
| `Section` | Panel-level groups; `OutlinerGroup` handles row-level groups |

---

## Recommended Build Order

1. **`StatusCell`** — trivial DOM component; no new data; can improve existing panels immediately
2. **`OutlinerRow`** — CSS grid row with named column slots; no data wiring yet
3. **`OutlinerGroup`** — collapsible group header using existing `Section` localStorage pattern
4. **ScenePanel** — first consumer; scene objects are the most intuitive outliner use in a 3D tool
5. **GpuPoolPanel** — second consumer; data already available from pool readback
6. **PerformancePanel pass rows** — third consumer; data from existing `frameTimeline`
7. **DebugPanel chunk rows** — blocked on per-chunk GPU readback; highest value, highest cost
8. **Cascade / lighting rows** — future; blocked on radiance cascade implementation

---

## Open Questions

- **Column responsiveness**: CSS container queries vs prop-driven column visibility?
- **Selection model**: Single-select only (current need), or multi-select with aggregate stats (future)?
- **Virtual scrolling**: At hundreds of chunks, DOM row count becomes a performance problem. Cap the list, or implement a virtualized scroller?
- **Keyboard navigation**: Arrow keys between rows (standard outliner UX). Interaction with existing `Section` keyboard handling?
- **Mode switching**: Does the panel show one domain at a time (like Blender's outliner modes), or are domains always in separate panels?
- **Write-back actions**: Visibility toggles in the scene outliner need to write back to the renderer. What is the correct command/event path for this?
