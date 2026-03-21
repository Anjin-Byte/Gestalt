import "@testing-library/jest-dom/vitest";

// happy-dom does not implement the Web Animations API.
// Svelte transitions (slide, fade, etc.) call element.animate() — stub it so
// tests involving animated state transitions don't throw.
if (typeof Element !== "undefined" && !Element.prototype.animate) {
  Element.prototype.animate = function () {
    return {
      cancel: () => {},
      finish: () => {},
      finished: Promise.resolve(undefined),
      addEventListener: () => {},
      removeEventListener: () => {},
    } as unknown as Animation;
  };
}
