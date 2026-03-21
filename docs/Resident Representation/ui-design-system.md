# Viaduct UI Design System

**Type:** spec
**Status:** stale
**Date:** 2026-03-21

its dark-mode-first "GitHub look", responsive layout, and component patterns.

---

## 1. Technology Stack

| Layer | Library | Version |
|---|---|---|
| Framework | Svelte | 5.x |
| Styling | Tailwind CSS | 4.x (Vite plugin) |
| Headless components | Bits UI | 2.x |
| Component system | shadcn-svelte | (registry pull) |
| Variant API | tailwind-variants (`tv()`) | 3.x |
| Class merging | clsx + tailwind-merge | — |
| Icons | Lucide Svelte | 0.577+ |
| Display font | Geist (variable) | 1.7+ |
| Mono font | Geist Mono (variable) | 1.7+ |
| Animation | tw-animate-css | 1.4+ |

**Key principle:** Bits UI provides the accessible, unstyled behavior layer. Tailwind classes applied via `tv()` variants provide all visual styling. This keeps logic and appearance fully separated.

---

## 2. Typography

### Fonts

```css
@font-face {
  font-family: "Geist";
  src: url("/assets/fonts/Geist-Variable.woff2") format("woff2");
  font-weight: 100 900;
  font-style: normal;
  font-display: swap;
}

@font-face {
  font-family: "Geist Mono";
  src: url("/assets/fonts/GeistMono-Variable.woff2") format("woff2");
  font-weight: 100 900;
  font-style: normal;
  font-display: swap;
}
```

Both fonts are self-hosted variable fonts (WOFF2). `font-display: swap` prevents flash-of-invisible-text.

### Theme mapping

```css
@theme inline {
  --font-sans: "Geist", system-ui, -apple-system, sans-serif;
  --font-mono: "Geist Mono", ui-monospace, monospace;
}
```

### Usage tiers

| Class | Use | Size |
|---|---|---|
| `.label` | Section labels, metadata keys | 11px, 500, uppercase, 0.05em tracking |
| body / `.value` | Primary content text | 14px, 400, 1.5 leading |
| `.meta` | Technical values, URLs, IDs | 12px, Geist Mono, break-all |
| `.value-muted` | Secondary content | 13px, muted color |
| `.section-header` | Section dividers | 12px, 600, 0.03em tracking, accent color |

### Rendering

```css
body {
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}
```

Antialiasing is forced on to match GitHub's crisp, subpixel-rendered look.

---

## 3. Color System

### Color model: OKLCH

All design tokens use the OKLCH color space: `oklch(Lightness Chroma Hue)`.

- **Perceptually uniform:** Equal L steps look equally different to the human eye
- **Hue consistency:** Chroma and hue are independent, so desaturating a color doesn't shift its perceived hue
- **Better dark mode:** Contrast ratios are predictable without empirical tweaking
- **Alpha support:** `oklch(0.22 0.015 250 / 55%)` for glass/translucent surfaces

The entire palette uses **hue 250°** (a cool blue-gray), keeping the UI monochromatic with a GitHub-like slate character.

---

### Light mode tokens

```css
:root {
  --radius: 0.625rem;                          /* 10px base radius */

  --background:           oklch(1 0 0);        /* pure white */
  --foreground:           oklch(0.129 0.042 264.695); /* near-black blue-tinted */

  --card:                 oklch(1 0 0);
  --card-foreground:      oklch(0.129 0.042 264.695);

  --popover:              oklch(1 0 0);
  --popover-foreground:   oklch(0.129 0.042 264.695);

  --primary:              oklch(0.208 0.042 265.755);  /* dark navy */
  --primary-foreground:   oklch(0.984 0.003 247.858);  /* off-white */

  --secondary:            oklch(0.968 0.007 247.896);  /* light gray */
  --secondary-foreground: oklch(0.208 0.042 265.755);

  --muted:                oklch(0.968 0.007 247.896);
  --muted-foreground:     oklch(0.554 0.046 257.417);  /* medium gray */

  --accent:               oklch(0.968 0.007 247.896);
  --accent-foreground:    oklch(0.208 0.042 265.755);

  --destructive:          oklch(0.577 0.245 27.325);   /* red */

  --border:               oklch(0.929 0.013 255.508);
  --input:                oklch(0.929 0.013 255.508);
  --ring:                 oklch(0.704 0.04 256.788);   /* focus ring */
}
```

---

### Dark mode tokens

The dark mode is the **primary experience** in Viaduct. It was designed with specific contrast ratios documented inline:

```css
.dark {
  /*
   * OKLCH contrast tuning against effective bg L≈0.18
   *
   * Body text:  L=0.96, C≈0 → ~13:1 contrast (AAA+), achromatic to avoid hue fatigue
   * Card text:  L=0.94, C≈0 → ~11:1 on card surface L≈0.22 (AAA)
   * Muted text: L=0.70, C=0.01 → ~5.5:1 on glass (AA), low chroma keeps it recessive
   * Primary:    L=0.72, C=0.11 → boosted L to offset H-K effect on saturated blue
   * Accent:     L=0.60, C=0.10 → interactive elements, AA compliant
   */

  --background:           oklch(0.16 0.015 250);       /* very dark blue-gray */
  --foreground:           oklch(0.96 0.003 250);       /* near-white, low chroma */

  --card:                 oklch(0.22 0.015 250 / 55%); /* glass card */
  --card-foreground:      oklch(0.94 0.003 250);

  --popover:              oklch(0.22 0.015 250 / 55%);
  --popover-foreground:   oklch(0.94 0.003 250);

  --primary:              oklch(0.72 0.11 250);        /* bright blue */
  --primary-foreground:   oklch(0.98 0.003 250);

  --secondary:            oklch(0.26 0.015 250 / 45%);
  --secondary-foreground: oklch(0.92 0.003 250);

  --muted:                oklch(0.26 0.015 250 / 35%);
  --muted-foreground:     oklch(0.70 0.01 250);        /* recessive gray */

  --accent:               oklch(0.60 0.10 250);        /* interactive blue */
  --accent-foreground:    oklch(0.98 0.003 250);

  --destructive:          oklch(0.68 0.18 25);         /* orange-red */

  --border:               oklch(1 0 0 / 12%);          /* white 12% */
  --input:                oklch(1 0 0 / 15%);
  --ring:                 oklch(0.72 0.11 250);

  --sidebar:              oklch(0.18 0.015 250);       /* slightly lighter than bg */
  --sidebar-foreground:   oklch(0.94 0.003 250);
  --sidebar-primary:      oklch(0.72 0.11 250);
  /* ... other sidebar tokens mirror main tokens */
}
```

---

### Border radius scale

```css
@theme inline {
  --radius-sm: calc(var(--radius) - 4px);  /* 6px */
  --radius-md: calc(var(--radius) - 2px);  /* 8px */
  --radius-lg: var(--radius);              /* 10px */
  --radius-xl: calc(var(--radius) + 4px);  /* 14px */
}
```

Use `rounded-md` (8px) for controls, `rounded-lg` (10px) for cards, `rounded-xl` (14px) for panels.

---

### Status/semantic colors

| Color | OKLCH | Purpose |
|---|---|---|
| Connected/success | `oklch(0.72 0.17 160)` | Green — active/online indicators |
| Warning | `oklch(0.76 0.12 80)` | Amber |
| Destructive | `oklch(0.68 0.18 25)` | Orange-red |
| Primary/interactive | `oklch(0.72 0.11 250)` | Blue — links, buttons, focus |
| Meta/code | `oklch(0.68 0.08 250)` | Blue-gray — monospace values |

---

## 4. Layout Architecture

### App shell

The outer container is always `.dark` class applied to the root element:

```
.dark
└── .scene                    ← full viewport, bg-background
    └── .layout               ← flex row, full height
        ├── .sidebar          ← 56px wide, dark column
        └── .surface          ← flex: 1, main content area
```

### Sidebar

```css
.sidebar {
  width: 56px;
  background: oklch(0.12 0.015 250);   /* darker than background */
  border-right: 1px solid rgba(255,255,255,0.06);
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  align-items: center;
  padding: 8px 0;
}
```

- **Icon-only by default** — labels appear as tooltips or below icons at small sizes
- Each item is `.sidebar-item`: 40×40px icon button, centered
- Active state: subtle background highlight + primary color icon
- Top area: brand/logo indicator + nav items
- Bottom area: utility items (Settings, Detach/popout)

### Status indicator

A small dot (6–8px circle) in the sidebar brand area signals connection status:

```css
.indicator {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: oklch(0.72 0.17 160);   /* green = connected */
  box-shadow: 0 0 6px oklch(0.72 0.17 160 / 60%);
}
```

### Surface / main content

```css
.surface {
  flex: 1;
  display: flex;
  flex-direction: column;
  backdrop-filter: blur(20px) saturate(1.3);   /* glass morphism */
  background: transparent;
  overflow: hidden;
}
```

The glass effect lets the dark background bleed through, creating depth between the sidebar and content layers.

---

## 5. Tab System

Tabs are the primary navigation within the surface area.

### Tab bar

```css
.tab-bar {
  display: flex;
  gap: 2px;
  padding: 6px 8px;
  border-bottom: 1px solid rgba(255,255,255,0.07);
  background: rgba(255,255,255,0.02);
  backdrop-filter: blur(8px);
}
```

### Tab trigger classes (from `tabs-trigger.svelte`)

```
data-[state=active]:bg-background
data-[state=active]:shadow-sm
dark:data-[state=active]:border-input
dark:data-[state=active]:bg-input/30
dark:data-[state=active]:text-foreground
dark:text-muted-foreground
inline-flex h-[calc(100%-1px)] flex-1 items-center justify-center
gap-1.5 rounded-md border border-transparent
px-2 py-1 text-sm font-medium whitespace-nowrap
transition-[color,box-shadow]
```

- Inactive: muted text, transparent background
- Active: slightly elevated card look (`bg-input/30` + shadow)
- Transition: color + box-shadow only (no layout shift)

### Tab list container

```
bg-muted text-muted-foreground inline-flex h-9 w-fit
items-center justify-center rounded-lg p-[3px]
```

---

## 6. Component Reference

### Button

**Import:** `import { Button } from "$lib/components/ui/button"`

**Variants:**

| Variant | Description |
|---|---|
| `default` | Primary action — `bg-primary`, white text |
| `destructive` | Delete/danger — red background |
| `outline` | Secondary action — bordered, transparent bg |
| `secondary` | Subtle — `bg-secondary` |
| `ghost` | Minimal — only hover state shows bg |
| `link` | Inline link style with underline on hover |

**Sizes:**

| Size | Height | Use |
|---|---|---|
| `default` | 36px (h-9) | Standard controls |
| `sm` | 32px (h-8) | Compact UI |
| `lg` | 40px (h-10) | Prominent CTAs |
| `icon` | 36×36px | Icon-only buttons |
| `icon-sm` | 32×32px | Compact icon buttons |
| `icon-lg` | 40×40px | Large icon buttons |

**Base classes (always applied):**
```
inline-flex shrink-0 items-center justify-center gap-2
rounded-md text-sm font-medium whitespace-nowrap
transition-all outline-none
focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]
disabled:pointer-events-none disabled:opacity-50
[&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4
```

Note the `[&_svg:not([class*='size-'])]:size-4` — icons auto-size to 16px unless the icon itself has a size class.

---

### Card

**Import:** `import * as Card from "$lib/components/ui/card"`

**Structure:**
```svelte
<Card.Root>
  <Card.Header>
    <Card.Title>Title</Card.Title>
    <Card.Description>Subtitle text</Card.Description>
    <Card.Action><!-- optional: action button --></Card.Action>
  </Card.Header>
  <Card.Content>
    <!-- body -->
  </Card.Content>
  <Card.Footer>
    <!-- footer actions -->
  </Card.Footer>
</Card.Root>
```

**Root classes:**
```
bg-card text-card-foreground flex flex-col gap-6
rounded-xl border py-6 shadow-sm
```

- Border uses `--border` token (white 12% in dark mode)
- `gap-6` (24px) between header/content/footer
- `py-6` vertical padding, content pads itself with `px-6`

**Header grid layout:**
```
@container/card-header grid auto-rows-min grid-rows-[auto_auto]
items-start gap-1.5 px-6
has-data-[slot=card-action]:grid-cols-[1fr_auto]
```

When `Card.Action` is present, the header becomes a two-column grid pushing the action to the right.

---

### Input

**Import:** `import { Input } from "$lib/components/ui/input"`

**Classes:**
```
border-input bg-background dark:bg-input/30
flex h-9 w-full min-w-0 rounded-md border
px-3 py-1 text-base md:text-sm
shadow-xs transition-[color,box-shadow] outline-none
placeholder:text-muted-foreground
focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]
disabled:cursor-not-allowed disabled:opacity-50
```

- Height: 36px (h-9), matching button default
- Focus: 3px ring in `--ring` color (primary blue in dark mode)
- Dark bg: `input/30` — the `--input` token at 30% opacity for the glass effect

---

### Badge

**Import:** `import { Badge } from "$lib/components/ui/badge"`

**Variants:**

| Variant | Appearance |
|---|---|
| `default` | `bg-primary`, white text, no border |
| `secondary` | `bg-secondary` muted |
| `destructive` | Red |
| `outline` | Transparent + border |

**Base:**
```
inline-flex w-fit shrink-0 items-center justify-center gap-1
overflow-hidden rounded-full border px-2 py-0.5
text-xs font-medium whitespace-nowrap
```

Pill shape (`rounded-full`), 12px text.

---

### Switch

**Import:** `import { Switch } from "$lib/components/ui/switch"`

```svelte
<Switch bind:checked={value} />
```

- Track: `h-[1.15rem] w-8` (18.4px × 32px)
- Thumb: `size-4` (16px circle)
- Checked: `bg-primary` track + thumb slides right
- Unchecked: `bg-input` track, thumb at left

---

### Label

**Import:** `import { Label } from "$lib/components/ui/label"`

```
flex items-center gap-2 text-sm leading-none font-medium select-none
group-data-[disabled=true]:pointer-events-none group-data-[disabled=true]:opacity-50
peer-disabled:cursor-not-allowed peer-disabled:opacity-50
```

Pairs with form controls via the `peer` pattern.

---

### Separator

**Import:** `import { Separator } from "$lib/components/ui/separator"`

```svelte
<Separator />                      <!-- horizontal -->
<Separator orientation="vertical" />
```

```
bg-border shrink-0
data-[orientation=horizontal]:h-px data-[orientation=horizontal]:w-full
data-[orientation=vertical]:min-h-full data-[orientation=vertical]:w-px
```

1px line using `--border` token.

---

## 7. Custom CSS Patterns

These classes in `shared.css` handle the content-list pattern used throughout popups:

### Content cards (data rows)

```css
.content-card {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 14px 0;
  border-bottom: 1px solid rgba(255, 255, 255, 0.05);
}
.content-card:last-child { border-bottom: none; }
```

Use for any key-value pair display: label stacked above value, separated by subtle dividers.

### Labels and values

```css
.label {
  font-size: 11px;
  font-weight: 500;
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: oklch(0.55 0.01 250);   /* dim gray */
}

.value {
  font-size: 14px;
  font-weight: 400;
  line-height: 1.5;
  color: oklch(0.94 0.003 250);  /* bright white */
}

.meta {
  font-family: var(--font-mono);
  font-size: 12px;
  line-height: 1.5;
  color: oklch(0.68 0.08 250);   /* blue-tinted mono color */
  word-break: break-all;
}
```

This `label → value → meta` three-tier hierarchy is the core data display pattern.

### Action buttons (inline/compact)

```css
.action-btn {
  font-size: 12px;
  font-weight: 500;
  color: oklch(0.98 0.003 250);
  background: oklch(0.35 0.04 250);
  border: 1px solid rgba(255, 255, 255, 0.10);
  border-radius: 6px;
  padding: 6px 14px;
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease;
}

.action-btn:hover:not(:disabled) {
  background: oklch(0.40 0.05 250);
  border-color: rgba(255, 255, 255, 0.18);
}

.action-btn:disabled { opacity: 0.5; cursor: not-allowed; }
```

Smaller than the full Button component — use for inline/compact contexts.

### Text buttons

```css
.text-btn {
  font-size: 12px;
  font-weight: 500;
  color: oklch(0.72 0.11 250);   /* primary blue */
  background: none;
  border: none;
  cursor: pointer;
  padding: 4px 8px;
  border-radius: 4px;
  transition: background 0.15s ease;
}

.text-btn:hover { background: rgba(255, 255, 255, 0.06); }
```

Ghost-style text action — for "Clear", "Reset", secondary actions.

### Form fields (native select/input)

When not using the component library (e.g., native `<select>`):

```css
.select-field {
  font-size: 13px;
  color: oklch(0.94 0.003 250);
  background: rgba(255, 255, 255, 0.04);
  border: 1px solid rgba(255, 255, 255, 0.10);
  border-radius: 6px;
  padding: 6px 10px;
  outline: none;
  transition: border-color 0.15s ease;
}

.select-field:focus { border-color: oklch(0.72 0.11 250); }

.select-field option {
  background: oklch(0.18 0.015 250);
  color: oklch(0.94 0.003 250);
}
```

---

## 8. Glass Morphism

Depth is created through layered translucency rather than solid backgrounds:

| Layer | Effect |
|---|---|
| Background | Solid `oklch(0.16 0.015 250)` |
| Cards | `oklch(0.22 0.015 250 / 55%)` — 45% transparent |
| Popovers | Same as card |
| Surface (main area) | `backdrop-filter: blur(20px) saturate(1.3)` |
| Tab bar | `backdrop-filter: blur(8px)` + `rgba(255,255,255,0.02)` |
| Borders | `rgba(255,255,255,0.05–0.12)` — white alpha only |

The pattern: **solid dark base → frosted glass panels → bright text on top**.

---

## 9. Interaction Patterns

### Focus rings

All interactive elements use:
```
focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]
```

- 3px offset ring using `--ring` token (primary blue)
- `focus-visible` only — keyboard users get indicator, mouse users don't

### Transitions

Standard transitions are brief and limited to visual properties only:

```css
transition: background 0.15s ease, border-color 0.15s ease;
/* or */
transition-[color,box-shadow]
/* or */
transition-all  /* on buttons */
```

No layout-affecting transitions (height, width, padding). The UI should feel instant.

### Disabled state

```
disabled:pointer-events-none disabled:opacity-50
```

50% opacity + no pointer events. Consistent across all interactive components.

### Hover states

Dark mode hover increments L by ~+0.05 and C by ~+0.01:

| State | Background |
|---|---|
| Rest | `oklch(0.35 0.04 250)` |
| Hover | `oklch(0.40 0.05 250)` |
| Border rest | `rgba(255,255,255,0.10)` |
| Border hover | `rgba(255,255,255,0.18)` |

---

## 10. Utility Function

All component and custom class composition uses this helper:

```typescript
import { clsx } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
```

Usage:
```svelte
<div class={cn("base-class", conditional && "extra-class", className)}>
```

`clsx` handles conditional logic; `twMerge` resolves Tailwind conflicts (e.g., two `bg-*` classes → last one wins).

---

## 11. Variant API Pattern

All reusable components with multiple styles use `tailwind-variants`:

```typescript
import { tv } from "tailwind-variants";

const buttonVariants = tv({
  base: "...shared classes...",
  variants: {
    variant: {
      default: "...",
      outline: "...",
    },
    size: {
      default: "...",
      sm: "...",
    }
  },
  defaultVariants: {
    variant: "default",
    size: "default",
  }
});
```

This gives full type safety and co-locates all visual variants in one place.

---

## 12. Dark Mode Activation

Dark mode is not toggled dynamically — the `.dark` class is applied to the root element at init:

```svelte
<!-- Popup.svelte root -->
<div class="dark scene">
  ...
</div>
```

To replicate in another project with toggle support:

```javascript
// Apply .dark class to <html> or root container
document.documentElement.classList.add("dark");

// Or toggle:
document.documentElement.classList.toggle("dark");
```

Tailwind 4 picks this up via the `@custom-variant dark (&:is(.dark *))` declaration in `app.css`.

---

## 13. Replication Checklist

To replicate this design in a new project:

- [ ] Install: `tailwindcss`, `@tailwindcss/vite`, `bits-ui`, `tailwind-variants`, `clsx`, `tailwind-merge`, `tw-animate-css`
- [ ] Add Geist and Geist Mono font files to assets
- [ ] Copy `app.css` entirely — the token definitions are the design system
- [ ] Set `@custom-variant dark (&:is(.dark *))` for dark mode
- [ ] Copy `src/lib/utils.ts` for the `cn()` helper
- [ ] Pull shadcn-svelte components via CLI or copy component files
- [ ] Apply `.dark` class to root element
- [ ] Use `data-slot` attributes on custom components for consistent targeting
- [ ] Stick to the OKLCH hue 250° palette for new colors to stay on-system
- [ ] Use `transition-[color,box-shadow]` not `transition-all` for performance

---

## 14. Settings / Form Layout Pattern

Settings panels follow this structure:

```
.settings-section          ← padding: 12px 0, border-bottom
├── Label                  ← .label (uppercase, 11px)
├── Input / Switch / Select
└── .setting-hint          ← 11px, oklch(0.50 0.01 250), optional hint text
```

```css
.settings-section {
  padding: 12px 0;
  border-bottom: 1px solid rgba(255, 255, 255, 0.04);
}
.settings-section:last-child { border-bottom: none; }

.setting-hint {
  display: block;
  font-size: 11px;
  color: oklch(0.50 0.01 250);
  margin-top: 4px;
}
```

Error text:
```css
.error-text {
  color: oklch(0.68 0.18 25);   /* destructive orange-red */
  font-size: 13px;
}
```
