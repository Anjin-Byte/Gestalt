import { render } from "@testing-library/svelte";
import { describe, it, expect } from "vitest";
import InlineSparkCell from "./InlineSparkCell.svelte";

describe("InlineSparkCell", () => {
  it("renders a .spark-cell wrapper", () => {
    const { container } = render(InlineSparkCell, { values: [1, 2, 3] });
    expect(container.querySelector(".spark-cell")).toBeInTheDocument();
  });

  it("renders the inner Sparkline canvas", () => {
    const { container } = render(InlineSparkCell, { values: [1, 2, 3] });
    expect(container.querySelector("canvas")).toBeInTheDocument();
  });

  it("sets canvas height to 14px", () => {
    const { container } = render(InlineSparkCell, { values: [1, 2, 3] });
    const canvas = container.querySelector("canvas") as HTMLCanvasElement;
    expect(canvas.style.height).toBe("14px");
  });

  it("renders with empty values", () => {
    const { container } = render(InlineSparkCell, {});
    expect(container.querySelector("canvas")).toBeInTheDocument();
  });
});
