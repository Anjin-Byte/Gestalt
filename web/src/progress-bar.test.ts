import * as fc from "fast-check";
import { describe, expect, it } from "vitest";

import { MESH_PHASES } from "./io-protocol";
import { segmentStates } from "./progress-bar";

describe("segmentStates", () => {
  it("is all-pending before the first event", () => {
    const states = segmentStates(MESH_PHASES, undefined);
    expect(states).toHaveLength(MESH_PHASES.length);
    expect(states.every((s) => s.kind === "pending")).toBe(true);
  });

  it("marks earlier phases done, the current active with its fraction", () => {
    const states = segmentStates(MESH_PHASES, {
      phase: "compact",
      done: 3,
      total: 4,
    });
    expect(states[0]).toEqual({ kind: "done" }); // parse
    expect(states[1]).toEqual({ kind: "done" }); // voxelize
    expect(states[2]).toEqual({ kind: "active", fraction: 0.75 });
    expect(states[3]).toEqual({ kind: "pending" }); // cutout
  });

  it("treats a zero total as indeterminate", () => {
    const states = segmentStates(MESH_PHASES, { phase: "parse", done: 0, total: 0 });
    expect(states[0]).toEqual({ kind: "active", fraction: undefined });
  });

  it("marks skipped phases done when a later phase reports", () => {
    // An untextured mesh never emits cutout, jumping voxelize→…→assemble.
    const states = segmentStates(MESH_PHASES, {
      phase: "assemble",
      done: 1,
      total: 10,
    });
    expect(states[3]).toEqual({ kind: "done" }); // cutout, skipped
  });

  it("clamps over-reporting to a full segment", () => {
    const states = segmentStates(MESH_PHASES, {
      phase: "voxelize",
      done: 9,
      total: 4,
    });
    expect(states[1]).toEqual({ kind: "active", fraction: 1 });
  });

  it("ignores an unknown phase rather than corrupting the bar", () => {
    const states = segmentStates(MESH_PHASES, {
      // A future kernel phase this build doesn't know.
      phase: "future" as never,
      done: 1,
      total: 2,
    });
    expect(states.every((s) => s.kind === "pending")).toBe(true);
  });

  // Property: for any known phase and any counts, the derivation is a
  // well-formed done*/active/pending* partition with an in-range fraction.
  it("always yields done-prefix, one active, pending-suffix for known phases", () => {
    fc.assert(
      fc.property(
        fc.nat({ max: MESH_PHASES.length - 1 }),
        fc.nat(),
        fc.nat(),
        (phaseIndex, done, total) => {
          const phase = MESH_PHASES[phaseIndex];
          if (phase === undefined) {
            throw new Error("unreachable: phaseIndex is bounded");
          }
          const states = segmentStates(MESH_PHASES, { phase, done, total });
          expect(states).toHaveLength(MESH_PHASES.length);
          for (const [i, state] of states.entries()) {
            const expected = i < phaseIndex ? "done" : i === phaseIndex ? "active" : "pending";
            expect(state.kind).toBe(expected);
          }
          const active = states[phaseIndex];
          if (active?.kind !== "active") {
            throw new Error("unreachable: checked above");
          }
          if (total === 0) {
            expect(active.fraction).toBeUndefined();
          } else {
            expect(active.fraction).toBeGreaterThanOrEqual(0);
            expect(active.fraction).toBeLessThanOrEqual(1);
          }
        },
      ),
      { seed: 42, numRuns: 300 },
    );
  });
});
