# Bug Report: Chrome/Firefox Layout Shift in DemoPanel

**Date:** 2026-03-20
**Status:** Open — root cause unconfirmed, all attempted fixes ineffective
**Browsers affected:** Chrome, Firefox
**Browser not affected:** Safari

---

## Symptom

When using the DemoPanel, enabling **Spike Mode** (or starting the simulation under certain conditions) causes everything in the panel to visually shift upward — into roughly the top third of the screen. The shift affects:

- All panel content elements
- The scrollbar
- Mouse selection highlights when click-dragging

The content is still interactive and the panel remains scrollable within the shifted/clipped region. The shift is **permanent until the page is reloaded**.

**Former workaround (now broken):** Collapsing and reopening the "TimelineCanvas" Section used to reset the layout. This workaround was inadvertently removed during a fix attempt (see Attempted Fixes below) and needs to be restored.

---

## Reproduction Steps

1. Navigate to the **Demo** tab in the panel.
2. Scroll down to the **TimelineCanvas** section.
3. Click **Start** to begin the simulation.
4. Enable **Spike Mode** via the checkbox.
5. Observe the panel content shift upward.

> **Note:** It is unclear whether Spike Mode itself is the trigger, or whether the trigger is the simulation starting (which causes `{#if demoDiag}` to mount for the first time). Spike Mode may be a red herring — the user may have noticed the shift coincidentally while enabling Spike Mode shortly after starting the simulation.

---

## What Spike Mode Actually Does

Enabling Spike Mode (`demoSpikeMode = true`) changes **only** the output of `makeDemoSample()` — 35% of frames get `totalMs += 6–22ms` instead of 4%. This affects:

- Canvas bar heights (more frames exceed 16ms budget → more red budget-exceeded overlays drawn)
- `PassBreakdownTable` values (same 8 rows, different averages over time)

**Spike Mode makes no structural DOM changes.** The CheckboxRow updates its checked state; nothing else in the DOM changes as a direct result of toggling the checkbox.

---

## Key Structural DOM Change: `{#if demoDiag}`

The one meaningful DOM change that occurs when the simulation runs is in `DemoPanel.svelte`:

```svelte
{#if demoDiag}
  <Section sectionId="demo-tc-diag" title="Synthetic Counters">
    ...
  </Section>
{/if}
```

`demoDiag` is `null` before the simulation starts and is set to a non-null object on the first `setInterval` tick (~50ms after clicking Start). This causes a **nested `<Section>` component to mount inside an already-open parent Section** — triggering a Svelte `slide` intro transition inside an already-sliding (or already-open) ancestor.

This is the most structurally significant event that correlates with the observed bug timing.

---

## Relevant Layout Hierarchy

```
.panel-area          (flex column, height: 100%, overflow: hidden)
  .panel-tab-content (flex column, height: 100%, overflow: hidden)
    .panel-content   (flex: 1, overflow-y: auto, padding: 12px)
      div.section    (border-bottom)
        button.section-trigger
        div.section-body  (transition:slide, overflow: visible, display: flex column)
          [content — including nested Sections]
```

`.panel-content` is the scroll container. It has `overflow-y: auto` and `flex: 1` inside a `display: flex` parent.

---

## What Has Been Ruled Out / Attempted

### Attempt 1: `overflow-anchor: none` on `.panel-content`

**Hypothesis:** Chrome/Firefox scroll anchoring was adjusting scroll position when new content (`{#if demoDiag}`) appeared, making everything appear to shift up.

**Result:** Did not fix the bug. The visual shift still occurs after adding `overflow-anchor: none` to `.panel-content` in `app.css`.

**Side effect:** CSS property remains in `app.css` — harmless, no behavior change.

---

### Attempt 2: Replace `{#if demoDiag} <Section>` with a plain `<div>`

**Hypothesis:** The nested Section's `slide` intro transition (mounting inside an already-open parent Section) was causing the Chrome/Firefox layout engine to re-measure the parent section's height mid-transition, producing an incorrect layout.

**Result:** Did not fix the bug.

**Side effect:** Removed the collapsible "Synthetic Counters" section from the demo. **Reverted** — the nested Section is back.

---

### Attempt 3: Replace Svelte `slide` with CSS `grid-template-rows` animation in `Section.svelte`

**Hypothesis:** Svelte's `slide` transition toggles the `.section-body` element between `overflow: hidden` (during animation) and `overflow: visible` (after animation). In Chrome/Firefox, this BFC (Block Formatting Context) toggle causes the browser to re-flow the scroll container geometry. Replacing the transition with a CSS `grid-template-rows: 0fr → 1fr` animation would keep `overflow: hidden` permanent (no BFC toggle) and eliminate the JS-driven height measurement.

**Result:** Did not fix the bug. Made it **worse** — the former "collapse to fix" workaround stopped working because removing `{#if open}` means Section children are always mounted (never unmounted on collapse).

**Side effect:** **Reverted** — Section is back to `{#if open}` + `transition:slide`.

---

## Current State of the Codebase

All attempted fixes have been reverted. The codebase is at its pre-investigation state with the following minor remnants:

- `overflow-anchor: none` remains on `.panel-content` in `app.css` (harmless)
- The orphaned `.demo-group-title` CSS rule was removed from `DemoPanel.svelte` (cleanup from Attempt 2)

---

## Open Questions

1. **Is the trigger `{#if demoDiag}` mounting, or Spike Mode itself?**
   Needs a controlled test: start the simulation and wait for `demoDiag` to appear *without* enabling Spike Mode. Does the shift occur?

2. **What exactly is visually shifting?**
   The description ("everything, all elements, scrollbar, even highlight when dragging") is consistent with either:
   - A CSS `transform` being applied to a parent container (which offsets hit-testing from visual position)
   - The scroll container's scroll position jumping (scroll anchoring)

   Since `overflow-anchor: none` didn't fix it, scroll anchoring seems less likely. The transform theory has not been directly tested.

3. **Why does collapsing the TimelineCanvas Section fix it (or used to)?**
   Collapsing triggers the `slide` outro on `.section-body`, which unmounts all children including the TimelineCanvas RAF loop and the nested Sections. Whatever state was corrupted is reset by this unmount/remount cycle.

4. **Is `flex: 1` without `min-height: 0` on `.panel-content` relevant?**
   Without `min-height: 0`, a flex item with `flex: 1` has `min-height: auto` (resolves to min-content height). This could allow `.panel-content` to grow beyond its flex container's bounds in edge cases. `.panel-tab-content`'s `overflow: hidden` would clip the visual result. Untested.

5. **Is there a CSS `transform` being applied somewhere dynamically?**
   Browser DevTools inspection during the bug state would confirm or deny this. A transform on any ancestor of `.panel-content` would explain why selection highlights are visually offset from mouse position.

---

## Recommended Next Steps

1. **DevTools investigation first.** Reproduce the bug, then inspect the computed styles and layout in Chrome DevTools with the bug active. Specifically:
   - Check for unexpected `transform` on any ancestor of `.panel-content`
   - Check the computed `height` and scroll position of `.panel-content`
   - Check whether `.section-body` has any unexpected inline styles left over from the `slide` transition

2. **Isolate the trigger.** Start the simulation, wait ~100ms for `demoDiag` to appear (without enabling Spike Mode). Determine if the shift happens at that moment.

3. **Test `min-height: 0`** on `.panel-content` to rule out the flex sizing hypothesis.

4. **Inspect the `slide` transition lifecycle** — specifically whether the inline `overflow: hidden` and `height` styles are being cleaned up correctly after the nested Section mounts.
