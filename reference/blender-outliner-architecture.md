# Blender Outliner — State Management Architecture

**Type:** reference
**Status:** current
**Date:** 2026-03-21
**Source:** `source/blender/editors/space_outliner/` in the Blender repository
**Purpose:** Technical reference for implementing an analogous outliner in Gestalt.

---

## Overview

Blender's Outliner solves a hard problem: the same panel must persist user state (collapse, selection) across full list rebuilds that happen every draw. The solution is a **two-tier data model** that separates persistent state from ephemeral display structure.

---

## Two-Tier Data Model

### Tier 1 — `TreeStoreElem` (persistent)

```c
typedef struct TreeStoreElem {
  short type;   /* TSE_* type code */
  short nr;     /* disambiguation index (e.g. slot 0 vs slot 1 for same ID) */
  short flag;   /* TSE_CLOSED | TSE_SELECTED | TSE_ACTIVE */
  short used;   /* scratch: 1 if this elem was matched during the current build */
  ID *id;       /* pointer to the Blender data-block */
} TreeStoreElem;
```

Stored in `SpaceOutliner.treestore`, a `BLI_mempool` — a fixed-block-size allocator that avoids per-element malloc overhead. This pool is **serialized to the `.blend` file**, so collapse and selection state survive save/load cycles.

The identity key for a `TreeStoreElem` is the triple `(id, type, nr)`. Two elements representing the same object at different rebuild cycles must produce the same key to restore their state.

### Tier 2 — `TreeElement` (ephemeral)

```c
typedef struct TreeElement {
  struct TreeElement *next, *prev, *parent;
  ListBase subtree;         /* child TreeElements */
  TreeStoreElem *store_elem; /* back-pointer to persistent state */
  const char *name;
  short idcode;
  /* ... display props, icon, xofs, ys ... */
} TreeElement;
```

A `TreeElement` is allocated per draw, linked into a temporary tree, then freed. It carries display data (name, icon, layout position) and a pointer back to the matching `TreeStoreElem`. It is never saved.

---

## Build / Draw / Sync Cycle

```
Every draw:
  1. outliner_build_tree()
     └─ Walks scene data, allocates TreeElement tree
     └─ For each element, looks up or creates a TreeStoreElem via (id, type, nr) hash
     └─ Marks matched TreeStoreElems as used=1
     └─ Unmatched entries (stale) are pruned from treestore

  2. RGN_DRAW_NO_REBUILD optimization
     └─ If only scroll position changed, skip step 1, redraw with existing tree

  3. outliner_draw_tree()
     └─ Walks TreeElement list, renders rows using TreeStoreElem.flag for state
```

The full rebuild every frame means the display is always consistent with scene state. There is no incremental update path to go stale.

---

## Collapse / Expand State

`TSE_CLOSED` is a bit flag in `TreeStoreElem.flag`. Because `TreeStoreElem` lives in the persisted mempool:

- Collapse state survives draw cycles, panel resize, mode switches, and file save/load.
- Toggling collapse = `store_elem->flag ^= TSE_CLOSED` + tag region for redraw.
- On rebuild, when a `TreeElement` is created, it checks its `TreeStoreElem.flag` to decide whether to recurse into children.

---

## Selection State

`TSE_SELECTED` and `TSE_ACTIVE` live in the same `TreeStoreElem.flag`. Selection is bidirectional:

- **Outliner → Scene**: clicking a row sets `TSE_SELECTED`, then calls into `BKE_scene_object_base_flag_sync_from_object()` to sync back to the object's base flags.
- **Scene → Outliner**: if the viewport selection changes, `wmWindowManager.outliner_sync_select_dirty` is set. On next outliner draw, `outliner_sync_selection_from_viewlayer()` walks the `TreeElement` tree and updates `TSE_SELECTED` flags before rendering.

This dirty-flag pattern avoids continuous polling. The outliner only syncs when the external selection actually changed.

---

## Filter System

`outliner_filter_tree()` is called after build, before draw.

- Uses `fnmatch()` for glob matching against element names.
- Sets `TSE_SEARCHMATCH` on matching rows.
- Sets `TSE_CHILDSEARCH` on ancestors of matching rows (so the path to the match stays visible).
- Non-matching rows without `TSE_CHILDSEARCH` are hidden by setting their `te->ys` to `-1`.

The filter does not rebuild the tree — it walks the existing `TreeElement` structure and annotates it. Fast, and avoids re-allocating the element pool.

---

## Display Mode Architecture

`SpaceOutliner.outlinevis` selects the active display mode (Objects, Data Blocks, Scenes, Sequence, Libraries, etc.). Each mode is implemented as a subclass of `AbstractTreeDisplay`:

```cpp
class AbstractTreeDisplay {
public:
  virtual ListBase buildTree(const TreeSourceData &source_data) = 0;
};

class TreeDisplayViewLayer : public AbstractTreeDisplay { ... };
class TreeDisplayLibraries : public AbstractTreeDisplay { ... };
class TreeDisplaySequencer : public AbstractTreeDisplay { ... };
// etc.
```

All modes produce the same `TreeElement` / `TreeStoreElem` output. The rest of the system (collapse, selection, filter, draw) is mode-agnostic. Switching mode = swapping the `AbstractTreeDisplay` subclass, rebuilding the tree.

---

## Key Design Decisions

| Decision | Why |
|---|---|
| `BLI_mempool` for `TreeStoreElem` | Fixed-block allocator; no per-element malloc; serializable |
| Full rebuild every draw | Simpler than incremental diff; always consistent with scene state |
| `(id, type, nr)` hash key | Stable identity across rebuilds without pointer equality |
| Dirty flag for selection sync | Avoids polling; syncs only when needed |
| Ephemeral `TreeElement` tree | Display data doesn't need to survive the frame |
| `AbstractTreeDisplay` subclass per mode | Same persistence/draw/filter infrastructure reused across all data domains |

---

## Implications for Gestalt

The Blender model maps cleanly onto the Outliner pattern described in `gpu-outliner-research.md`.

**Persistent state store** → `OutlinerState` object (or `localStorage`) keyed by `(domain, id, type)`. Stores collapse, selection, pinned. Survives panel remounts and page reloads.

**Ephemeral display rows** → Svelte's reactive `$derived` list. Computed from live data each frame; no cache to go stale. Reads from `OutlinerState` for flag bits during render.

**Full rebuild on data change** → Natural in Svelte's reactive model. `$derived` arrays recompute when their dependencies change. No incremental patch needed.

**Dirty flag for selection sync** → A writable store `selectedId` that both the panel and the renderer subscribe to. Panel sets it on row click; renderer sets it when the camera focuses an object.

**`AbstractTreeDisplay` per domain** → Separate data-mapping functions (`sceneRows()`, `gpuPoolRows()`, `chunkRows()`) that produce a common `OutlinerRowData[]` type, consumed by shared `OutlinerRow` / `OutlinerGroup` components.

---

## Source Files for Reference

| File | Purpose |
|---|---|
| `space_outliner.cc` | `SpaceOutliner` struct, space type registration |
| `outliner_tree.cc` | `outliner_build_tree()`, `TreeElement` allocation |
| `outliner_draw.cc` | Row rendering, icon/name layout, column drawing, highlight pass |
| `outliner_select.cc` | Click handling, `TSE_SELECTED` updates, scene sync |
| `outliner_filter.cc` | `outliner_filter_tree()`, `TSE_SEARCHMATCH` |
| `outliner_dragdrop.cc` | Drag initiation, insert-zone logic, reparent execution |
| `outliner_ops.cc` | All operator registrations |
| `outliner_utils.cc` | `outliner_right_columns_width()` per display mode |
| `tree/tree_display.hh` | `AbstractTreeDisplay` base class |
| `tree/tree_display_view_layer.cc` | Default Object mode display |
| `editors/interface/interface_icons.cc` | `icon_from_library()` — linked/override/missing icons |

---

## Restriction Columns

The inline eye/viewport/render toggles on the right side of each row — the defining feature of the Blender Outliner pattern.

### Data structures

There is no descriptor table mapping icon → property → callback. Instead two parallel structs carry the state:

```cpp
struct RestrictProperties {
  bool initialized;
  // Object
  PropertyRNA *object_hide_viewport, *object_hide_select, *object_hide_render;
  PropertyRNA *base_hide_viewport;
  // Collection / LayerCollection
  PropertyRNA *collection_hide_viewport, *collection_hide_select, *collection_hide_render;
  PropertyRNA *layer_collection_exclude, *layer_collection_holdout,
              *layer_collection_indirect_only, *layer_collection_hide_viewport;
  // Modifier / Constraint / Bone
  PropertyRNA *modifier_show_viewport, *modifier_show_render;
  PropertyRNA *constraint_enable;
  PropertyRNA *bone_hide_viewport;
};

struct RestrictPropertiesActive {
  // Same field list — bool per property.
  // True = column interactive. False = grayed (inherited parent-collection state).
  bool object_hide_viewport;
  // ...
};
```

`RestrictProperties` is a static local in `outliner_draw_restrictbuts()`, initialized once via `RNA_struct_type_find_property()`.

### Column set per display mode

Column visibility is controlled by `space_outliner->show_restrict_flags` (a `char` bitmask):

| Bit | Column | Mode restriction |
|---|---|---|
| `SO_RESTRICT_ENABLE` | Collection "exclude from view layer" | `SO_VIEW_LAYER` only |
| `SO_RESTRICT_SELECT` | Disable selection | all |
| `SO_RESTRICT_HIDE` | Hide in viewport (view-layer local) | all |
| `SO_RESTRICT_VIEWPORT` | Globally hide in viewports | all |
| `SO_RESTRICT_RENDER` | Hide from render | all |
| `SO_RESTRICT_HOLDOUT` | Holdout | `SO_VIEW_LAYER` only |
| `SO_RESTRICT_INDIRECT_ONLY` | Indirect only | `SO_VIEW_LAYER` only |

`outliner_right_columns_width()` in `outliner_utils.cc` returns `0.0f` for `SO_DATA_API`, `SO_SEQUENCE`, and `SO_LIBRARIES` — those modes have no columns at all.

### How clicks work

Columns are plain `uiDefIconButR_prop()` buttons. The UI button system calls `RNA_property_boolean_set()` on click — no custom operator needed. Each button gets a post-change callback:

- Object properties → `outliner__object_set_flag_recursive_fn`
- Base properties → `outliner__base_set_flag_recursive_fn`
- Collection → `view_layer__collection_set_flag_recursive_fn`
- LayerCollection → `view_layer__layer_collection_set_flag_recursive_fn`

Restriction columns are protected from triggering row selection: `outliner_item_do_activate_from_cursor()` calls `outliner_is_co_within_restrict_columns()` and returns `OPERATOR_CANCELLED` if the x-coordinate lands in the column area.

### Propagate to children (Shift)

`outliner_object_set_flag_recursive_fn()` reads `win->runtime->eventstate->modifier & KM_SHIFT`. Without Shift it returns immediately. With Shift it iterates `bmain->objects`, tests `BKE_object_is_child_recursive()`, and applies `RNA_property_boolean_set()` to each child. Button tooltip encodes the hint: `"…\n • Shift to set children"`.

---

## Row Color Coding

Color coding operates at two independent layers: background (selection/hover/drag) and text (object state, hidden/faded).

### Background — `outliner_draw_highlights()`

A separate pre-pass reads `TreeStoreElem.flag` bits and draws rounded rectangles:

| Condition | Color token |
|---|---|
| `TSE_ACTIVE \| TSE_SELECTED` | `TH_SELECT_ACTIVE` |
| `TSE_SELECTED` only | `TH_SELECT_HIGHLIGHT` |
| `TSE_SEARCHMATCH` | `TH_MATCH` at 0.5 alpha |
| `TSE_HIGHLIGHTED` (hover) | white at 0.13 alpha |
| `TSE_DRAG_INTO` | `TH_BACK` + 40 shade + border |
| `TSE_DRAG_BEFORE` | top border line at `TH_TEXT` blended with `TH_BACK` at 0.4 |
| `TSE_DRAG_AFTER` | bottom border line (same) |

### Text color — `outliner_draw_tree_element()`

Base color is `TH_TEXT`. The conditional chain:

1. Object (`TSE_SOME_ID`, `ID_OB`): active+selected → `TH_ACTIVE_OBJECT`; selected → `TH_SELECTED_OBJECT`
2. Object data in edit mode: icon background `TH_EDITED_OBJECT`
3. Active scene: `TH_TEXT_HI`
4. Active non-scene TSE: `TH_TEXT_HI`
5. RNA properties: blend `TH_BACK` → `TH_TEXT` at 0.75 (grayed)
6. `element_should_draw_faded()` → alpha_fac = 0.5 (the "hidden/excluded" graying)

`element_should_draw_faded()` checks: `BASE_ENABLED_AND_VISIBLE_IN_DEFAULT_VIEWPORT` for objects; `LAYER_COLLECTION_VISIBLE_VIEW_LAYER` and `LAYER_COLLECTION_EXCLUDE` for collections.

### Library/linked state — icon, not color

There is **no blue or red text color** for linked/missing data. Status is communicated by a small secondary icon drawn after the name via `icon_from_library()`:

| ID state | Icon |
|---|---|
| Linked, packed | `ICON_PACKAGE` |
| Linked, missing | `ICON_LIBRARY_DATA_BROKEN` |
| Linked, indirect | `ICON_LIBRARY_DATA_INDIRECT` |
| Linked, direct | `ICON_LIBRARY_DATA_DIRECT` |
| Library override (system) | `ICON_LIBRARY_DATA_OVERRIDE_NONEDITABLE` |
| Library override (editable) | `ICON_LIBRARY_DATA_OVERRIDE` |
| Asset | `ICON_ASSET_MANAGER` |

This icon appears on `TSE_SOME_ID`, `TSE_LAYER_COLLECTION`, and `TSE_LINKED_NODE_TREE` elements.

---

## Drag-and-Drop Reparenting

### Drop boxes

Five drop operators are registered in `outliner_dropboxes()`:

| Operator | Purpose |
|---|---|
| `OUTLINER_OT_parent_drop` | Reparent object to another object |
| `OUTLINER_OT_parent_clear` | Clear parent by dropping onto empty space |
| `OUTLINER_OT_scene_drop` | Move object to a different scene |
| `OUTLINER_OT_material_drop` | Assign material to object slot |
| `OUTLINER_OT_datastack_drop` | Reorder modifiers/constraints/bones |
| `OUTLINER_OT_collection_drop` | Move object into a collection |

### Insert-zone three-way split — `outliner_drop_insert_find()`

Each row is divided by `margin = UI_UNIT_Y / 4`:

| Mouse Y position | Zone | Visual |
|---|---|---|
| Bottom quarter (`< ys + margin`) | `TE_INSERT_AFTER` | bottom border line |
| Top quarter (`> ys + 3*margin`) | `TE_INSERT_BEFORE` | top border line |
| Middle half | `TE_INSERT_INTO` | background box |

When `TE_INSERT_AFTER` lands on an open element with children, the insert redirects to `TE_INSERT_BEFORE` on the first child.

### Reparent execution

```cpp
// parent_drop_set_parents() in outliner_dragdrop.cc
object::parent_set(reports, C, scene, object, parent, parent_type,
                   false, keep_transform, nullptr);
// → BKE_object_parent_type_check → sets ob->parent, ob->partype
// → DEG_relations_tag_update(bmain)
// → NC_OBJECT | ND_TRANSFORM + NC_OBJECT | ND_PARENT notifiers
```

Undo is handled by the operator flags `OPTYPE_REGISTER | OPTYPE_UNDO` — no explicit `ED_undo_push()`.

### Display mode restrictions

```cpp
static bool allow_parenting_without_modifier_key(SpaceOutliner *space_outliner) {
  switch (space_outliner->outlinevis) {
    case SO_VIEW_LAYER: return space_outliner->filter & SO_FILTER_NO_COLLECTION;
    case SO_SCENES:     return true;
    default:            return false;
  }
}
```

In any other mode, `parent_drop_poll()` requires Shift to be held. Collection drops use `OUTLINER_OT_collection_drop` with `BKE_collection_object_move()` / `BKE_collection_object_add()`.

---

## Active vs. Selected — Two-State Selection Model

### Definitions

Both flags live in `TreeStoreElem.flag`. They are independent bits:

- **`TSE_SELECTED`** — this element is part of the selection set. Maps to `BASE_SELECTED` on the object's view layer base, `BONE_SELECTED`, `SEQ_SELECT`, etc.
- **`TSE_ACTIVE`** — this element is the single most-recently-touched element. Maps to `ViewLayer.objects.active` / `BKE_view_layer_active_object_get()`, `ebone_active`, `pchan_active`, `seq::select_active_get()`.

This mirrors the 3D viewport exactly: many objects can be selected, only one is active.

### Gesture → Flag Mapping

From `outliner_item_do_activate_from_cursor()` and `outliner_item_select()`:

| Gesture | Result |
|---|---|
| Plain click | Clears all `TSE_SELECTED` + `TSE_ACTIVE` from tree, sets **both** on the clicked element |
| Shift-click | Range select from current `TSE_ACTIVE` to cursor — sets `TSE_SELECTED` on range; `TSE_ACTIVE` does **not** move |
| Ctrl-click (extend) | Toggles `TSE_SELECTED` and `TSE_ACTIVE` on the clicked element without clearing others |
| Rubber-band / box select | Sets `TSE_SELECTED` only on all enclosed elements; `TSE_ACTIVE` is never updated |
| Arrow-key walk | Always sets **both** `TSE_SELECTED` and `TSE_ACTIVE` on the destination element |

The key line (outliner_select.cc:1571):
```cpp
const short clear_flag = (activate ? TSE_ACTIVE : 0) | (extend ? 0 : TSE_SELECTED);
```
Box select passes only `OL_ITEM_SELECT | OL_ITEM_EXTEND` — no `OL_ITEM_ACTIVATE` — so it never touches `TSE_ACTIVE`.

### ACTIVE without SELECTED — a real state

This is a real, producible state. The sync code in `outliner_sync.cc` sets the two flags independently:

```cpp
if (base && (ob == obact)) {
    tselem->flag |= TSE_ACTIVE;
} else {
    tselem->flag &= ~TSE_ACTIVE;
}
if (is_selected) {
    tselem->flag |= TSE_SELECTED;
} else {
    tselem->flag &= ~TSE_SELECTED;
}
```

In Blender's viewport, Alt+A deselects all objects but keeps the active object. The outliner faithfully mirrors this: `TSE_ACTIVE=1, TSE_SELECTED=0`.

Visually, ACTIVE-without-SELECTED draws **no background** — there is no draw branch for `TSE_ACTIVE` alone. The element appears unselected. Only the combination `TSE_ACTIVE | TSE_SELECTED` produces the bright accent color.

### Visual states

```cpp
// outliner_draw.cc:3750
if ((tselem->flag & TSE_ACTIVE) && (tselem->flag & TSE_SELECTED)) {
    draw_roundbox(col_active);          // filled TH_SELECT_ACTIVE + lighter outline
}
else if (tselem->flag & TSE_SELECTED) {
    draw_roundbox(col_selection);       // filled TH_SELECT_HIGHLIGHT
}
// TSE_ACTIVE only: nothing drawn
```

| State | Visual |
|---|---|
| ACTIVE + SELECTED | Filled `TH_SELECT_ACTIVE` (orange/accent) + lighter outline |
| SELECTED only | Filled `TH_SELECT_HIGHLIGHT` (muted blue/grey) |
| ACTIVE only | No background — visually identical to unselected |
| Neither | No background |

### Per-instance vs. global

`TSE_ACTIVE` and `TSE_SELECTED` live in `SpaceOutliner->treestore`, which is per-editor-instance. However, the sync system re-derives both flags from the global `ViewLayer.objects.active` and `BASE_SELECTED` on every tree rebuild, so all outliner instances converge to the same state after the next redraw. The storage is per-instance; the represented state is global.

---

## Scroll-to-Active / Ensure Visible

### The operator: `OUTLINER_OT_show_active`

Scroll-to-active is an **explicit user-triggered operator**, not automatic per-frame behavior. Defined in `outliner_edit.cc`.

**`outliner_show_active_exec()`**:
1. Finds the active element via `BKE_view_layer_active_object_get()` + tree walk.
2. Calls `outliner_open_back(te)` recursively to expand any collapsed ancestors of the active element.
3. Recomputes Y positions via `outliner_set_coordinates()`.
4. Computes scroll target: `ytop = active_element->ys + (region_height / 2)`, then `delta_y = ytop - v2d->cur.ymax`. This always **centers** the active element regardless of whether it is already visible.
5. Calls `outliner_scroll_view(space_outliner, region, delta_y)` — an immediate, non-animated `v2d->cur` mutation.

### What triggers scroll-to-active

| Trigger | Scrolls? | Notes |
|---|---|---|
| Viewport selection change (`NC_SCENE\|ND_OB_ACTIVE`) | **No** | Notifier causes redraw/highlight update only |
| User presses `.` / View → Show Active | **Yes** | `OUTLINER_OT_show_active` explicit operator |
| Keyboard walk (arrow keys) | **Yes, conditionally** | `outliner_walk_scroll()` — only when destination is outside the visible Y range; scrolls to boundary, not center |
| Inline rename triggered | **Yes, conditionally** | Scrolls if active element is within 1 `UI_UNIT_Y` of the top or bottom edge |

**There is no automatic scroll when the viewport selection changes.** The outliner will highlight the newly active row correctly (via the redraw notifier + sync), but it will not scroll to make it visible. The user must press `.` explicitly.

### `outliner_scroll_view()` mechanics

```cpp
// outliner_utils.cc
void outliner_scroll_view(SpaceOutliner *space_outliner, ARegion *region, int delta_y)
{
    region->v2d.cur.ymax += delta_y;
    region->v2d.cur.ymin += delta_y;
    // Clamped to [tree_bottom, -UI_UNIT_Y]
}
```

- No animation — instant `v2d.cur` mutation within the current frame.
- No partial-visibility check in the `show_active` path — always recenters.
- Walk-scroll checks partial visibility before calling it.
- Scroll position is per `ARegion` (per-outliner-instance).

---

## Implications for Gestalt (expanded)

| Blender mechanism | Gestalt equivalent |
|---|---|
| `RestrictProperties` + `uiDefIconButR_prop` | `OutlinerRow` column slots accept any Svelte snippet — restriction cells are just `<button>` elements bound to a store write |
| `show_restrict_flags` bitmask | Per-domain column config object passed to the `OutlinerGroup`; columns rendered conditionally |
| Shift-propagate to children | `onToggle(id, value, propagate: boolean)` callback; propagation logic lives in the domain data layer, not the row component |
| Click guard (`outliner_is_co_within_restrict_columns`) | `stopPropagation()` on restriction cell click events prevents row selection |
| `TH_SELECT_HIGHLIGHT` / `TH_SELECT_ACTIVE` | CSS `--interactive-fill` tint on row `selected` prop; two-tone via `--interactive-fill-active` |
| `element_should_draw_faded()` + alpha 0.5 | `faded` prop on `OutlinerRow` → `opacity: 0.5` on name + value cells |
| `icon_from_library()` secondary icon | Secondary icon slot in `OutlinerRow` after the name; domain data layer passes the icon token |
| `TE_INSERT_BEFORE/INTO/AFTER` three-way zone | `onDragOver` computes zone from `event.offsetY / rowHeight`; sets drag-target class on row |
| `OPTYPE_REGISTER | OPTYPE_UNDO` on reparent | Scene mutation goes through a command object that is pushed to an undo stack |
