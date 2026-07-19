// @vitest-environment happy-dom
// BuildBar DOM-writer tests: the thin shell over the vitest-covered
// segmentStates derivation. The oracle is the exact DOM the CSS styles —
// segment classes and fill widths. The bar rebuilds its segments per
// begin(phases), so it serves any job's phase list (mesh, fixture, decode,
// encode); these cover both a long list and a short one.
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ENCODE_PHASES, MESH_PHASES } from "./io-protocol";
import { BuildBar, FINISH_LINGER_MS } from "./progress-bar";

let root: HTMLElement;
let bar: BuildBar;

function seg(i: number): HTMLElement {
  const el = root.children[i];
  if (!(el instanceof HTMLElement)) {
    throw new Error(`no segment ${i}`);
  }
  return el;
}

function fillWidth(i: number): string {
  const fill = seg(i).firstElementChild;
  if (!(fill instanceof HTMLElement)) {
    throw new Error(`no fill ${i}`);
  }
  return fill.style.width;
}

beforeEach(() => {
  document.body.innerHTML = "";
  root = document.createElement("div");
  root.hidden = true; // as in index.html
  document.body.appendChild(root);
  bar = new BuildBar(root);
});

describe("BuildBar", () => {
  it("has no segments until a job begins", () => {
    expect(root.children).toHaveLength(0);
  });

  it("builds one titled segment per phase of the job it is given, in order", () => {
    bar.begin(MESH_PHASES);
    expect(root.children).toHaveLength(MESH_PHASES.length);
    expect([...root.children].map((c) => (c as HTMLElement).title)).toEqual([
      "parse",
      "voxelize",
      "compact",
      "cutout",
      "assemble",
      "colorBake",
      "pack",
      "install",
    ]);
  });

  it("rebuilds its segment set when a different job begins", () => {
    bar.begin(MESH_PHASES);
    expect(root.children).toHaveLength(8);
    bar.begin(ENCODE_PHASES); // a 2-phase job must not keep the 8 mesh segments
    expect(root.children).toHaveLength(2);
    expect([...root.children].map((c) => (c as HTMLElement).title)).toEqual(["gather", "write"]);
  });

  it("begin shows an all-pending bar (no stale fills from the last job)", () => {
    bar.begin(MESH_PHASES);
    bar.update({ phase: "assemble", done: 1, total: 2 });
    bar.end();
    bar.begin(MESH_PHASES); // a new job must not inherit the old fills
    expect(root.hidden).toBe(false);
    for (let i = 0; i < MESH_PHASES.length; i += 1) {
      expect(fillWidth(i)).toBe("0.0%");
      expect(seg(i).classList.contains("active")).toBe(false);
    }
  });

  it("update fills earlier phases, marks the active one, and leaves the rest empty", () => {
    bar.begin(MESH_PHASES);
    bar.update({ phase: "compact", done: 3, total: 4 });
    expect(fillWidth(0)).toBe("100.0%"); // parse: done
    expect(fillWidth(1)).toBe("100.0%"); // voxelize: done
    expect(fillWidth(2)).toBe("75.0%"); // compact: active fraction
    expect(seg(2).classList.contains("active")).toBe(true);
    expect(seg(2).classList.contains("indeterminate")).toBe(false);
    expect(fillWidth(3)).toBe("0.0%"); // cutout: pending
    expect(seg(3).classList.contains("active")).toBe(false);
  });

  it("meters a real-count phase in a short job's bar (encode gather)", () => {
    bar.begin(ENCODE_PHASES);
    bar.update({ phase: "gather", done: 1, total: 4 });
    expect(fillWidth(0)).toBe("25.0%");
    expect(seg(0).classList.contains("active")).toBe(true);
    bar.update({ phase: "write", done: 0, total: 0 });
    expect(fillWidth(0)).toBe("100.0%"); // gather done
    expect(seg(1).classList.contains("indeterminate")).toBe(true); // write pulses
  });

  it("marks a zero-total phase indeterminate (pulse, not a fake fraction)", () => {
    bar.begin(MESH_PHASES);
    bar.update({ phase: "parse", done: 0, total: 0 });
    expect(seg(0).classList.contains("active")).toBe(true);
    expect(seg(0).classList.contains("indeterminate")).toBe(true);
    // No inline width: the stylesheet's indeterminate rule (full-width pulse)
    // must own it — an inline 0% would render the segment invisibly empty.
    expect(fillWidth(0)).toBe("");
    bar.update({ phase: "parse", done: 1, total: 2 });
    expect(seg(0).classList.contains("indeterminate")).toBe(false); // determinate again
    expect(fillWidth(0)).toBe("50.0%");
  });

  it("finish fills every segment, lingers, then auto-hides", () => {
    vi.useFakeTimers();
    try {
      bar.begin(ENCODE_PHASES);
      bar.update({ phase: "write", done: 0, total: 0 }); // ends on a pulse
      bar.finish();
      // Completed state visible: everything full, nothing active/pulsing.
      expect(root.hidden).toBe(false);
      for (let i = 0; i < ENCODE_PHASES.length; i += 1) {
        expect(fillWidth(i)).toBe("100.0%");
        expect(seg(i).classList.contains("active")).toBe(false);
        expect(seg(i).classList.contains("indeterminate")).toBe(false);
      }
      vi.advanceTimersByTime(FINISH_LINGER_MS);
      expect(root.hidden).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });

  it("a new job during the finish linger cancels the pending hide", () => {
    vi.useFakeTimers();
    try {
      bar.begin(ENCODE_PHASES);
      bar.finish();
      bar.begin(MESH_PHASES); // next job starts before the linger elapses
      vi.advanceTimersByTime(FINISH_LINGER_MS * 2);
      expect(root.hidden).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it("end hides the bar", () => {
    bar.begin(MESH_PHASES);
    bar.end();
    expect(root.hidden).toBe(true);
  });
});
