# Gestalt Outliner — Implementation Spec

**Type:** spec
**Status:** proposed
**Date:** 2026-03-21
**Scope:** Component API, state model, domain adapter contract, column system, selection protocol, drag-and-drop, filter, keyboard navigation, responsive layout.

---

## Goals

- A single set of primitive components (`OutlinerRow`, `OutlinerGroup`) that every panel domain (scene, GPU resources, render passes, chunks, lighting) consumes without modification.
- Inline state columns — visible and mutable in the same gesture, without opening a sub-panel.
- Persistent collapse and selection state that survives panel remounts and page reloads.
- Bidirectional selection sync between the outliner and the renderer.
- Zero coupling between the display components and any specific domain's data shape.

## Non-Goals

- Full virtual scrolling (capped list with overflow indicator for now; virtual scrolling is a separate task).
- Multi-select (single-select only in v1, but the state model supports it — see Selection Model below).
- Undo history for outliner state changes (selection, collapse). Undo applies to scene mutations only.

---

## Architecture Overview

```
Domain data (store)
      │
      ▼
OutlinerDomain<T>.rows(data)     ← domain adapter: maps T → OutlinerItem[]
      │
      ▼
Outliner.svelte                  ← owns filter bar, keyboard handler, scroll container
  ├─ OutlinerGroup.svelte        ← collapsible group header row
  └─ OutlinerRow.svelte          ← single data row with CSS grid columns
        ├─ StatusCell            ← dot + optional short label
        └─ InlineSparkCell       ← fixed-width Sparkline wrapper
```

The domain adapter is a plain object — not a Svelte component. It converts domain data into a flat array of typed display items. The `Outliner` component renders that array. Hierarchy is encoded as `depth` on each row item, not as a nested tree structure. This keeps rendering and reactivity simple.

---

## TypeScript Interfaces

### Display item types

```typescript
/** A flat list of these is the domain adapter's output. */
export type OutlinerItem = OutlinerGroupItem | OutlinerRowItem;

export interface OutlinerGroupItem {
  kind: 'group';
  /** Stable ID — used as the collapse persistence key. */
  id: string;
  label: string;
  /** Optional aggregate displayed in the group header (e.g. a BarMeter). */
  aggregate?: { value: number; max: number; unit?: string };
  count?: number;
}

export interface OutlinerRowItem {
  kind: 'row';
  /** Stable ID — used as the selection key. */
  id: string;
  /** Group this row belongs to (determines whether it is visible when group is collapsed). */
  groupId: string;
  /** Indentation depth. 0 = top-level. Each level = 12px left indent. */
  depth?: number;
  /** Lucide icon name, or undefined for no icon. */
  icon?: string;
  label: string;
  /**
   * Secondary icon rendered after the name. Used for status badges that
   * modify the label's meaning (e.g. linked, missing, override).
   */
  statusBadge?: StatusBadge;
  /**
   * At 0.5 opacity — communicates "excluded from active context"
   * (hidden object, excluded collection, disabled modifier, etc.).
   */
  faded?: boolean;
  /** Values for each column slot, in the same order as OutlinerDomain.columns. */
  cells: OutlinerCellData[];
  draggable?: boolean;
}

export type StatusBadge =
  | 'linked'
  | 'linked-indirect'
  | 'linked-missing'
  | 'override'
  | 'override-system'
  | 'asset';

export type OutlinerCellData =
  | { type: 'status'; status: 'ok' | 'warning' | 'error' | 'idle'; label?: string }
  | { type: 'mono';   value: string }
  | { type: 'spark';  values: number[]; warn?: number; danger?: number }
  | { type: 'toggle'; value: boolean; icon: string; disabled?: boolean; propagatable?: boolean };
```

### Column definition

```typescript
export interface OutlinerColumnDef {
  /** Unique ID within the domain. Passed to onToggle for toggle cells. */
  id: string;
  /** Fixed width in px. The name column takes all remaining flex space. */
  width: number;
  /**
   * Panel width (in px) below which this column is hidden.
   * Implemented with CSS container queries — no JS needed.
   * Omit for columns that are always visible.
   */
  hideBelow?: number;
  /** Accessible label shown in the column header tooltip. */
  label: string;
}
```

### Domain adapter

```typescript
export interface OutlinerDomain<T> {
  /** Unique domain ID — used to namespace localStorage keys. */
  domainId: string;
  /** Column definitions. Order must match OutlinerRowItem.cells order. */
  columns: OutlinerColumnDef[];
  /**
   * Maps current domain data to a flat list of display items.
   * Called reactively via $derived — must be a pure function.
   */
  rows(data: T): OutlinerItem[];
  /**
   * Called when the user clicks a row. The domain decides what "select" means:
   * focusing a scene object, highlighting a GPU buffer slot, etc.
   */
  onSelect?(id: string): void;
  /**
   * Called when the user clicks a toggle cell.
   * propagate = true when the user held Shift during the click.
   * The domain is responsible for applying propagation to children.
   */
  onToggle?(rowId: string, columnId: string, value: boolean, propagate: boolean): void;
  /**
   * Called when the user drops a dragged row onto a target row.
   * The domain is responsible for the actual mutation and any undo push.
   */
  onDrop?(dragId: string, targetId: string, zone: DropZone): void;
}

export type DropZone = 'before' | 'into' | 'after';
```

---

## Persistent State

### What is persisted

| State | Storage | Lifetime |
|---|---|---|
| Collapsed groups | `localStorage` | Cross-session |
| Selected row IDs | Svelte writable store | Session only (page lifetime) |
| Active row ID | Svelte writable store | Session only (page lifetime) |
| Filter string | Component-local `$state` | Transient (clears on remount) |
| Scroll position | `scrollTop` restored on mount | Session only |

### OutlinerStateStore

A small class that wraps localStorage for collapse state. One instance per domain, keyed by `domainId`.

```typescript
class OutlinerStateStore {
  private key: string;
  private collapsed: Set<string>;

  constructor(domainId: string) {
    this.key = `outliner:${domainId}:collapsed`;
    const saved = localStorage.getItem(this.key);
    this.collapsed = new Set(saved ? JSON.parse(saved) : []);
  }

  isCollapsed(groupId: string): boolean {
    return this.collapsed.has(groupId);
  }

  toggle(groupId: string): void {
    if (this.collapsed.has(groupId)) {
      this.collapsed.delete(groupId);
    } else {
      this.collapsed.add(groupId);
    }
    localStorage.setItem(this.key, JSON.stringify([...this.collapsed]));
  }
}
```

This follows the same pattern as `Section.svelte`'s `localStorage.getItem(\`panel-section:\${sectionId}\`)` — initialized synchronously on mount, written on each toggle, no Svelte store overhead.

### Selection model — Active vs. Selected

Gestalt uses the same two-state model as Blender's outliner:

- **Selected** — a row is in the selection set. Multiple rows can be selected simultaneously (v1 exposes only one at a time, but the store supports sets).
- **Active** — the single most-recently-clicked row. There is exactly one active row (or none). Active drives scene focus: the renderer highlights the active object, the Properties panel shows active-object properties.

The two states are independent:
- A row can be active without being selected (e.g. after deselecting all but keeping focus).
- A row can be selected without being active (e.g. during range select — the anchor stays active).
- Visually: **active + selected** → accent background; **selected only** → muted background; **active only** → no background.

The shared store:

```typescript
// stores/outlinerSelection.ts
import { writable } from 'svelte/store';

export interface OutlinerSelectionState {
  domainId: string;
  /** The set of selected row IDs. */
  selected: Set<string>;
  /** The single active row ID, or null. Active need not be in selected. */
  activeId: string | null;
}

export const outlinerSelection = writable<OutlinerSelectionState | null>(null);
```

### Gesture → selection state mapping

| Gesture | Effect on `selected` | Effect on `activeId` |
|---|---|---|
| Click | Clear all, add clicked | Set to clicked |
| Shift-click | Add range from `activeId` to clicked | No change |
| Ctrl-click | Toggle clicked in set | Set to clicked |
| Arrow key walk | Set to destination only | Set to destination |
| Escape | Clear all | Set to null |

In v1 (single-select), the Shift and Ctrl variants behave the same as plain click. The table documents the intended v2 behavior so the store shape doesn't need to change.

The `Outliner` component writes to `outlinerSelection` on row interaction. The renderer subscribes and highlights the active object. When the renderer's own selection changes (viewport click), it writes to `outlinerSelection` with the new `activeId`. The `Outliner` reads `outlinerSelection` via `$effect` and reflects it as row highlight — but does **not** automatically scroll to the newly active row (see Scroll-to-Active below).

---

## Component API

### `OutlinerRow`

```svelte
<!-- Props -->
let {
  item,        // OutlinerRowItem
  columns,     // OutlinerColumnDef[]
  selected,    // boolean — row is in the selection set
  active,      // boolean — row is the single active row
  dragTarget,  // DropZone | null — which insert-zone is active during a drag
  onclick,     // (id: string, event: MouseEvent) => void
  ontoggle,    // (rowId: string, columnId: string, value: boolean, propagate: boolean) => void
  ondragstart, // (id: string, event: DragEvent) => void
  ondragover,  // (id: string, zone: DropZone) => void
  ondrop,      // (id: string) => void
}: OutlinerRowProps = $props();
```

**CSS grid layout.** The row is a single CSS grid. The name track is `1fr`; each column track is its declared `width` in px.

```css
.outliner-row {
  display: grid;
  /* grid-template-columns set inline from column widths: "1fr 16px 48px 52px" */
  align-items: center;
  height: 22px;
  padding-left: calc(var(--depth, 0) * 12px);
  border-radius: 3px;
  cursor: pointer;
  user-select: none;
}

/* Three independent visual states — matches Blender's TH_SELECT_ACTIVE / TH_SELECT_HIGHLIGHT model */
.outliner-row:hover                      { background: oklch(1 0 0 / 0.07); }
.outliner-row.selected                   { background: var(--interactive-fill); }
.outliner-row.selected.active            { background: var(--interactive-fill-active);
                                           outline: 1px solid var(--interactive-fill); }
/* Active-only (not selected): no background — visually identical to unselected.
   The cursor's position in the list is tracked without a selection halo. */

.outliner-row.drag-into   { background: var(--fill-lo); outline: 1px solid var(--stroke-strong); }
.outliner-row.drag-before { border-top: 2px solid var(--text-mid); }
.outliner-row.drag-after  { border-bottom: 2px solid var(--text-mid); }
.outliner-row.faded .ol-name { opacity: 0.5; }
```

**Column cells** are rendered from `item.cells` in order. Toggle cells call `event.stopPropagation()` on click to prevent triggering row selection — the same guard as Blender's `outliner_is_co_within_restrict_columns()`.

**Expand chevron.** Placed in the name track's left gutter (before the icon), not in a separate column. Groups handle their own chevron; rows that are themselves expandable (e.g. a render pass with child resource rows) get the same chevron in the name track.

### `OutlinerGroup`

```svelte
let {
  item,        // OutlinerGroupItem
  stateStore,  // OutlinerStateStore
  columns,     // OutlinerColumnDef[] — used for column header alignment
}: OutlinerGroupProps = $props();
```

The group header row uses the same CSS grid as `OutlinerRow` so column headers align. The name track contains the chevron + group label. The rightmost track renders the aggregate (`BarMeter`) if `item.aggregate` is set; the count badge appears just left of the aggregate.

Collapse state is read from and written to `stateStore`. The `slide` transition from `svelte/transition` at `duration: 140` matches `Section.svelte`.

### `Outliner`

The container component. Owns:
- The `OutlinerStateStore` instance (one per domain, created on mount)
- The filter input `$state`
- Keyboard event handler on the scroll container
- The rendered flat list (`$derived` from domain + data + collapse state + filter)

```svelte
let {
  domain,    // OutlinerDomain<T>
  data,      // T — reactive; updated from the parent's store subscription
  maxRows,   // number — default 200; rows beyond this show an overflow indicator
}: OutlinerProps = $props();
```

**Derived render list:**

```typescript
const allItems = $derived(domain.rows(data));

const visibleItems = $derived((() => {
  const collapsed = /* stateStore collapsed set */;
  const filter = filterString.trim().toLowerCase();

  return allItems.filter(item => {
    if (item.kind === 'group') return true;

    // Collapse: hide rows whose group is collapsed
    if (collapsed.has(item.groupId)) return false;

    // Filter: hide rows that don't match the filter string
    if (filter && !item.label.toLowerCase().includes(filter)) return false;

    return true;
  });
})());
```

Filter on groups: if a group header passes through but all its children are filtered out, the group header is also hidden. If any children match, the group header is shown and the group is force-expanded.

### `StatusCell`

A fixed-width column cell wrapping `StatusIndicator`. The existing `StatusIndicator` component is used directly — `StatusCell` is just a layout wrapper that constrains width and centers the dot.

```svelte
<!-- Used inside OutlinerRow column slots -->
<div class="status-cell">
  <StatusIndicator {status} {label} />
</div>

<style>
  .status-cell {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
  }
</style>
```

### `InlineSparkCell`

A fixed-width column cell wrapping `Sparkline`. Height is 14px (tighter than the standalone 24px sparkline) to fit within the 22px row height with 4px vertical padding.

```svelte
<div class="spark-cell">
  <Sparkline values={cell.values} height={14} warn={cell.warn} danger={cell.danger} />
</div>

<style>
  .spark-cell { width: 100%; }
</style>
```

---

## Column System

### Defining columns for a domain

Each domain exports a column definition array alongside its `rows()` function:

```typescript
// Example: Scene domain columns
const sceneColumns: OutlinerColumnDef[] = [
  { id: 'viewport', width: 20, label: 'Hide in viewport' },
  { id: 'render',   width: 20, label: 'Disable render',  hideBelow: 300 },
];

// Example: GPU pool domain columns
const gpuPoolColumns: OutlinerColumnDef[] = [
  { id: 'status',  width: 20, label: 'Buffer status' },
  { id: 'version', width: 44, label: 'Version', hideBelow: 260 },
  { id: 'size',    width: 44, label: 'Size (KB)', hideBelow: 320 },
  { id: 'spark',   width: 52, label: 'Age history', hideBelow: 340 },
];
```

### Responsive column hiding

The `Outliner` container uses a CSS container query:

```css
.outliner {
  container-type: inline-size;
  container-name: outliner;
}
```

Each column with a `hideBelow` value gets a generated CSS rule:

```css
@container outliner (max-width: 299px) {
  .ol-col[data-hide-below="300"] { display: none; }
}
```

The `grid-template-columns` value on each row is computed at runtime from the currently visible column widths. This requires a `$derived` that filters `columns` by the container width. Use a `ResizeObserver` on the container to track width in a `$state`.

### Toggle column click guard

In the `OutlinerRow` component, toggle cells are `<button>` elements. Their `onclick` handler calls `event.stopPropagation()` before invoking `ontoggle`. This prevents the row's own `onclick` (which sets selection) from firing — exactly the `outliner_is_co_within_restrict_columns()` guard.

```svelte
<!-- Inside OutlinerRow, for a toggle cell -->
<button
  class="ol-toggle"
  onclick={(e) => {
    e.stopPropagation();
    ontoggle?.(item.id, col.id, !cell.value, e.shiftKey);
  }}
  aria-label={col.label}
  aria-pressed={cell.value}
>
  <!-- icon -->
</button>
```

The `e.shiftKey` check mirrors Blender's `KM_SHIFT` propagation check. The domain's `onToggle` handler receives `propagate = e.shiftKey` and decides whether to walk children.

---

## Drag-and-Drop

### Three-zone insert model

Each `OutlinerRow` has a `dragover` handler that computes the drop zone from the mouse's Y offset within the row:

```typescript
function computeZone(event: DragEvent, el: HTMLElement): DropZone {
  const rect = el.getBoundingClientRect();
  const relY = event.clientY - rect.top;
  const h = rect.height;
  if (relY < h * 0.25) return 'before';
  if (relY > h * 0.75) return 'after';
  return 'into';
}
```

Visual feedback per zone:
- `before` → `border-top: 2px solid var(--text-mid)`
- `after` → `border-bottom: 2px solid var(--text-mid)`
- `into` → `background: var(--fill-lo); outline: 1px solid var(--stroke-strong)`

### Drag lifecycle

The `Outliner` container holds `dragSourceId: string | null` and `dropTarget: { id: string; zone: DropZone } | null` as `$state`. On `dragend`, the domain's `onDrop` is called with `(dragSourceId, dropTarget.id, dropTarget.zone)`. The domain executes the mutation and the state resets.

Domains that don't support drag-and-drop omit `onDrop` from their adapter object. `OutlinerRow` checks `item.draggable` to decide whether to set `draggable="true"` on the row element.

---

## Filter

The filter input is a plain `<input type="search">` at the top of the `Outliner`. Filtering is case-insensitive substring match on `item.label`. It does not search cell values.

**Group behavior during filter:**
- A group with no matching children is hidden entirely.
- A group with matching children is shown and force-expanded (collapse state is ignored while filter is active).
- Matching rows have their label text wrapped in a `<mark>` element for visual highlight.

Filter state is not persisted — clearing the filter (Escape or empty input) restores the natural collapsed/expanded state.

---

## Keyboard Navigation

The `Outliner` container listens for `keydown` on its scroll container (`tabindex="0"`).

| Key | Action |
|---|---|
| `ArrowDown` | Move selection to the next visible row, skipping group headers |
| `ArrowUp` | Move selection to the previous visible row |
| `ArrowRight` | If on a collapsed group: expand. If on a row with children: expand. Otherwise: no-op. |
| `ArrowLeft` | If on an expanded group: collapse. If inside a group: move focus to the group header. Otherwise: no-op. |
| `Enter` | Confirm selection (same as click) |
| `Escape` | Clear filter if active; otherwise deselect |

The active row index is tracked as a `$state` index into `visibleItems`. On active change via keyboard, the row element calls `scrollIntoView({ block: 'nearest' })` — this scrolls the minimum amount needed to bring the row into view, not to center it.

Keyboard navigation does not interact with `Section.svelte`'s keyboard handling because `Outliner` captures events on its own container before they bubble.

---

## Scroll-to-Active

### When scrolling happens

| Trigger | Scroll behavior |
|---|---|
| User clicks a row | No scroll — the clicked row is already visible by definition |
| Keyboard walk (arrow keys) | `scrollIntoView({ block: 'nearest' })` — minimum scroll to bring destination into view |
| External selection change (viewport click) | **No automatic scroll** — the row highlights, but the list does not move |
| User invokes "Show Active" action | Centers the active row: scrolls `activeRowEl.scrollIntoView({ block: 'center' })` |

The deliberate choice not to auto-scroll on external selection changes mirrors Blender's behavior: the outliner highlights the newly active row but does not move the viewport. This prevents disorienting jumps when the user is browsing a long list and clicks in the 3D viewport. The user can invoke "Show Active" (`F` or a toolbar button) to find the active item.

### Show Active implementation

```typescript
function showActive(activeId: string) {
  // 1. Force-expand all collapsed ancestors of the active row
  //    (walk allItems to find ancestors by groupId chain)
  forceExpandAncestors(activeId);

  // 2. Find the DOM element for the active row
  const el = rowElements.get(activeId);
  if (!el) return;

  // 3. Center it — not nearest, center
  el.scrollIntoView({ block: 'center', behavior: 'smooth' });
}
```

`rowElements` is a `Map<string, HTMLElement>` populated by `bind:this` on each rendered row element. `forceExpandAncestors` temporarily overrides the collapsed state for any group that contains the active row, then triggers a re-render before scrolling.

`behavior: 'smooth'` is used here (unlike keyboard walk which uses instant `nearest`) because Show Active is an intentional navigation gesture, not a rapid keystroke. A short smooth scroll provides spatial orientation without blocking interaction.

### Scroll position persistence

The scroll container's `scrollTop` is saved to a module-level variable on `beforeunload` and restored on mount:

```typescript
// On mount, after first render
scrollEl.scrollTop = savedScrollTop[domain.domainId] ?? 0;

// On destroy
savedScrollTop[domain.domainId] = scrollEl.scrollTop;
```

This keeps the list position stable across panel collapse/expand and hot module reloads during development. Not persisted to localStorage — scroll position is session-only.

---

## Relationship to Existing Components

| Existing component | Role after Outliner |
|---|---|
| `PropRow` | Unchanged. Still used for scalar key/value pairs that don't need scan-at-a-glance layout. |
| `BarMeter` | Promoted to `OutlinerGroup` aggregate header. Not replaced. |
| `CounterRow` / `Sparkline` | `Sparkline` is reused inside `InlineSparkCell`. `CounterRow` is used outside outliners (GPU Diagnostics section). |
| `StatusIndicator` | Reused inside `StatusCell`. |
| `Section` | Panel-level collapsible groups. `OutlinerGroup` handles row-level groups within a section. They do not overlap. |

---

## Build Order

1. **`StatusCell`** — trivial wrapper around existing `StatusIndicator`; no data wiring; drop-in improvement to existing panels immediately.

2. **`OutlinerRow`** — CSS grid row with cell rendering from `OutlinerCellData[]`; no state, no selection logic; testable in `DemoPanel` with static data.

3. **`OutlinerGroup`** — collapsible header using `OutlinerStateStore`; same localStorage pattern as `Section.svelte`; no domain wiring yet.

4. **`Outliner`** — container combining filter, collapse, keyboard, selection store write; accepts a generic `OutlinerDomain<T>`.

5. **`InlineSparkCell`** — extracted from `CounterRow` pattern; 14px height.

6. **ScenePanel** — first live consumer. Scene object list is the most intuitive outliner use. Wires `outlinerSelection` store to the renderer's object focus.

7. **GpuPoolPanel** — second consumer. Buffer slot data already available from pool readback.

8. **PerformancePanel** — third consumer. Render pass rows from `frameTimeline`.

9. **DebugPanel chunk rows** — blocked on per-chunk GPU readback. Highest value, highest infrastructure cost.

---

## Open Questions

| Question | Current answer | Deferred? |
|---|---|---|
| Virtual scrolling | Cap at 200 rows, show "(N more hidden)" indicator | Yes — implement when chunk count exceeds cap in practice |
| Multi-select | Store shape supports it (Set<string>); v1 UI is single-select only | Yes — future, with aggregate stats in group header |
| Active vs. Selected distinction | Both tracked separately; active drives renderer focus, selected drives visual highlight | Decided |
| Auto-scroll on external selection change | No — highlights only; user invokes Show Active explicitly | Decided |
| Mode switching | Domains are always in separate panels (not Blender-style mode switching) | Decided |
| Write-back path for scene mutations | `onSelect` / `onToggle` / `onDrop` on `OutlinerDomain` — domain handles the mutation | Decided |
| Keyboard focus when panel is inside a scrollable sidebar | `tabindex="0"` on `.outliner` container; focus on first click | Needs UX validation |
| Container query polyfill | Not needed — all target browsers support `@container` | Decided |
