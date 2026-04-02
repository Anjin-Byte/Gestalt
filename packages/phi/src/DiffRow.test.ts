import { render } from "@testing-library/svelte";
import { describe, it, expect } from "vitest";
import DiffRow from "./DiffRow.svelte";

describe("DiffRow", () => {
  it("renders label, prev, and current values", () => {
    const { getByText } = render(DiffRow, {
      label: "Frame time", prev: 8, current: 12,
    });
    expect(getByText("Frame time")).toBeInTheDocument();
    expect(getByText("8")).toBeInTheDocument();
    expect(getByText("12")).toBeInTheDocument();
  });

  it("shows arrow separator", () => {
    const { getByText } = render(DiffRow, {
      label: "L", prev: 1, current: 2,
    });
    expect(getByText("→")).toBeInTheDocument();
  });

  it("shows positive delta with + prefix", () => {
    const { container } = render(DiffRow, {
      label: "L", prev: 10, current: 18,
    });
    const delta = container.querySelector(".diff-delta");
    expect(delta?.textContent).toContain("+8");
  });

  it("shows negative delta", () => {
    const { container } = render(DiffRow, {
      label: "L", prev: 20, current: 15,
    });
    const delta = container.querySelector(".diff-delta");
    expect(delta?.textContent).toContain("-5");
  });

  it("shows zero delta with neutral class", () => {
    const { container } = render(DiffRow, {
      label: "L", prev: 42, current: 42,
    });
    const delta = container.querySelector(".diff-delta");
    expect(delta?.textContent).toContain("0");
    expect(delta).toHaveClass("diff-neutral");
  });

  it("applies unit suffix to prev and current", () => {
    const { container } = render(DiffRow, {
      label: "L", prev: 8.5, current: 12.3, unit: "ms", decimals: 1,
    });
    const prev = container.querySelector(".diff-prev");
    const curr = container.querySelector(".diff-current");
    expect(prev?.textContent).toContain("8.5");
    expect(prev?.textContent).toContain("ms");
    expect(curr?.textContent).toContain("12.3");
    expect(curr?.textContent).toContain("ms");
  });

  it("respects decimals prop", () => {
    const { getByText } = render(DiffRow, {
      label: "L", prev: 3.14159, current: 2.71828, decimals: 2,
    });
    expect(getByText("3.14")).toBeInTheDocument();
    expect(getByText("2.72")).toBeInTheDocument();
    expect(getByText("-0.42")).toBeInTheDocument();
  });

  it("positive delta gets warning class by default", () => {
    const { container } = render(DiffRow, {
      label: "L", prev: 5, current: 10,
    });
    expect(container.querySelector(".diff-warning")).toBeInTheDocument();
    expect(container.querySelector(".diff-good")).not.toBeInTheDocument();
  });

  it("negative delta gets good class by default", () => {
    const { container } = render(DiffRow, {
      label: "L", prev: 10, current: 5,
    });
    expect(container.querySelector(".diff-good")).toBeInTheDocument();
  });

  it("invertWarning flips: negative delta is warning", () => {
    const { container } = render(DiffRow, {
      label: "FPS", prev: 60, current: 45, invertWarning: true,
    });
    // FPS dropped — should be warning
    expect(container.querySelector(".diff-warning")).toBeInTheDocument();
  });

  it("invertWarning flips: positive delta is good", () => {
    const { container } = render(DiffRow, {
      label: "FPS", prev: 45, current: 60, invertWarning: true,
    });
    expect(container.querySelector(".diff-good")).toBeInTheDocument();
  });

  it("zero delta gets neutral class", () => {
    const { container } = render(DiffRow, {
      label: "L", prev: 100, current: 100,
    });
    expect(container.querySelector(".diff-neutral")).toBeInTheDocument();
    expect(container.querySelector(".diff-warning")).not.toBeInTheDocument();
    expect(container.querySelector(".diff-good")).not.toBeInTheDocument();
  });
});
