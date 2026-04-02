# Phi

Opinionated GUI toolkit for building professional tool interfaces. Designed for CG tools, GPU debuggers, voxel editors — applications that need dense, information-rich panels with DCC-tool conventions. Not a generic component library. Not a design system for web apps.

## Design Principles

**Dense by default.** Small fonts (11-12px), tight spacing, high information density. Every pixel carries data. Panels should look like Blender's properties editor, not a marketing page.

**Composition over configuration.** Small, focused components that compose into panels. A `Section` contains `PropRow`s, `BarMeter`s, `CounterRow`s — each does one thing. Panels are built by stacking these atoms, not by configuring a mega-component.

**Real-time data first.** Components are designed for data that updates every frame. `CounterRow` shows a value + sparkline history. `DiffRow` shows before/after deltas. `BarMeter` animates fill smoothly. If your data is static, `PropRow` is fine — but most data in a professional tool isn't static.

**Accessible.** ARIA roles, keyboard navigation, screen reader labels. Every interactive component has a keyboard path. Focus rings are visible. This isn't optional — it's built in.

**Themeable via CSS variables.** Colors use `--text-hi`, `--text-muted`, `--fill-lo`, `--stroke-mid`, `--interactive`, etc. Switch themes by redefining variables on a parent element.

**Svelte 5 runes.** All components use `$state`, `$derived`, `$props()`. No legacy Svelte 4 stores or reactive declarations.

---

## Component Taxonomy

### Display (read-only data)

| Component | What | When to use |
|-----------|------|------------|
| **PropRow** | Label + monospace value with copy button | Static or infrequently changing values |
| **CounterRow** | Label + value + inline sparkline | Values that change every frame — show the trend |
| **DiffRow** | Prev → current with delta arrow and trending indicator | Comparing two states: CPU vs GPU, frame N vs N-1 |
| **BarMeter** | Horizontal fill bar with threshold warning | Ratios: pool usage, capacity, progress toward a limit |
| **Sparkline** | Canvas-based mini line chart | Standalone history visualization (usually inside CounterRow) |
| **StatusIndicator** | Colored dot (ok/warning/error/idle) with optional pulse | Boolean health: is this subsystem running? |
| **BitField** | Row of labeled on/off/unknown pills | Packed flags: chunk state bits, pipeline feature flags |

### Input (user controls)

| Component | What | When to use |
|-----------|------|------------|
| **ToggleGroup** | Segmented radio buttons, always visible | 2-4 mutually exclusive choices (render mode, view type) |
| **SelectField** | Dropdown with chevron | 5+ options, or when horizontal space is limited |
| **CheckboxRow** | Labeled toggle | Enable/disable a feature, debug overlay, option |
| **ScrubField** | Drag-to-scrub numeric input with step buttons | Fine control: exposure, threshold, camera parameter |
| **Slider** | Simple range input | Exploring a numeric range without precision needs |
| **ActionButton** | Styled button with optional icon | One-shot actions: reset, pause, export, frame object |

### Layout (structure)

| Component | What | When to use |
|-----------|------|------------|
| **Section** | Collapsible container with localStorage persistence | Grouping related controls in a panel |
| **DockLayout** | Absolute-positioned dock panel renderer | Application shell — wraps the entire panel grid |
| **DockGroup** | Tabbed panel container | Multiple panels sharing one dock zone |
| **DockTabs** | Tab bar with close buttons | Tab navigation within a DockGroup |
| **Splitview** | 1D resizable split pane | Dividing space between two views |
| **Gridview** | Recursive tree of alternating-orientation splits | Complex multi-panel layouts |

### Complex (interactive data views)

| Component | What | When to use |
|-----------|------|------------|
| **TreeList** | Hierarchical list with columns, sort, drag-drop, inline edit | Scene graphs, chunk inventories, resource lists |
| **ContextMenu** | Fixed-position floating menu with keyboard nav | Right-click actions on specific items |

---

## Component Selection Guide

Use this decision tree when building a panel. Start from the data you need to show:

```
What are you showing?
|
+-- A label + value
|   +-- Value rarely changes             --> PropRow
|   +-- Value changes every frame
|   |   +-- Want history trend            --> CounterRow (sparkline)
|   |   +-- Want before/after comparison  --> DiffRow (delta arrow)
|   |   +-- Just the number              --> PropRow (reactive binding)
|   +-- Value is a ratio (current / max) --> BarMeter
|
+-- A boolean state (on/off/healthy)
|   +-- Single state                      --> StatusIndicator
|   +-- Multiple flags in a row           --> BitField
|
+-- A small inline chart                  --> Sparkline
|
+-- User picks one of N options
|   +-- 2-4 options, always visible       --> ToggleGroup
|   +-- 5+ options or tight space         --> SelectField
|
+-- User toggles a feature               --> CheckboxRow
|
+-- User adjusts a number
|   +-- Fine control (drag, step, type)   --> ScrubField
|   +-- Rough exploration (range)         --> Slider
|
+-- User triggers an action              --> ActionButton
+-- Group of related controls            --> Section
+-- Hierarchical data list               --> TreeList
+-- Right-click actions                   --> ContextMenu
```

---

## Anti-patterns

These are common mistakes. Avoid them.

**Using PropRow for a value that changes every frame.** PropRow has no history, no trend indicator. If the value updates frequently, use `CounterRow` (shows sparkline) or `DiffRow` (shows delta). PropRow is for labels like "Chunk Coord: (0,0,0)" — not for "Frame Time: 16.3ms".

**Using SelectField for 3 render modes.** SelectField hides the options behind a dropdown click. With 2-4 options, use `ToggleGroup` — all options are visible at once, one click to switch. SelectField is for long lists (material palette, shader variant).

**Hardcoding values that exist in program state.** Don't write `<PropRow label="FOV" value="45°" />`. Bind to the store: `value={`${fovDeg}°`}`. If the value comes from a constant, compute it from the constant, don't duplicate the number.

**Using static strings for pool sizes.** Don't write `value="256 MB"`. Compute from the actual buffer size or constant. If MAX_VERTS_PER_CHUNK changes, hardcoded strings silently become wrong.

**Building a custom component when a Phi component fits.** Before creating a one-off progress bar, check if `BarMeter` does what you need. Before making a flag display, check `BitField`. Phi components are tested and themed consistently.

---

## Theming

All components use CSS custom properties defined on a parent element (typically the `<body>` with a `.dark` class). Key variables:

| Variable | Purpose |
|----------|---------|
| `--text-hi` | Primary text (labels, values) |
| `--text-muted` | Secondary text (units, hints) |
| `--text-faint` | Tertiary text (disabled, placeholder) |
| `--fill-lo` | Subtle background fills |
| `--fill-mid` | Medium background fills |
| `--stroke-mid` | Borders, dividers |
| `--interactive` | Interactive element accent |
| `--surface-1` | Panel background |
| `--surface-2` | Elevated panel background |
| `--font-sans` | UI font family |
| `--font-mono` | Monospace font (values, code) |

Components never hardcode colors. They reference these variables, so switching themes is a single CSS class change.
