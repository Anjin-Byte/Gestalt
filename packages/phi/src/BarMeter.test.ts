import { render } from "@testing-library/svelte";
import { describe, it, expect, beforeEach } from "vitest";
import BarMeter from "./BarMeter.svelte";

describe("BarMeter", () => {
  it("renders label and default value display", () => {
    const { getByText } = render(BarMeter, { label: "Slots", value: 50, max: 100 });
    expect(getByText("Slots")).toBeInTheDocument();
    expect(getByText("50 / 100")).toBeInTheDocument();
  });

  it("formats display value with unit", () => {
    const { getByText } = render(BarMeter, { label: "L", value: 128, max: 256, unit: "MB" });
    expect(getByText("128 / 256 MB")).toBeInTheDocument();
  });

  it("uses valueLabel override when provided", () => {
    const { getByText } = render(BarMeter, { label: "L", value: 50, max: 100, valueLabel: "custom display" });
    expect(getByText("custom display")).toBeInTheDocument();
  });

  it("no tier class below warning threshold (50%)", () => {
    const { container } = render(BarMeter, { label: "L", value: 50, max: 100 });
    const fill = container.querySelector(".bar-fill");
    expect(fill).not.toHaveClass("warn");
    expect(fill).not.toHaveClass("crit");
  });

  it("applies warn class at warning tier — default threshold 80%", () => {
    const { container } = render(BarMeter, { label: "L", value: 85, max: 100 });
    const fill = container.querySelector(".bar-fill");
    expect(fill).toHaveClass("warn");
    expect(fill).not.toHaveClass("crit");
  });

  it("applies crit class at critical tier (≥90%)", () => {
    const { container } = render(BarMeter, { label: "L", value: 92, max: 100 });
    const fill = container.querySelector(".bar-fill");
    expect(fill).toHaveClass("crit");
    expect(fill).not.toHaveClass("warn");
  });

  it("exactly at threshold boundary is warning not crit", () => {
    const { container } = render(BarMeter, { label: "L", value: 80, max: 100 });
    const fill = container.querySelector(".bar-fill");
    expect(fill).toHaveClass("warn");
  });

  it("exactly at 90% is critical", () => {
    const { container } = render(BarMeter, { label: "L", value: 90, max: 100 });
    const fill = container.querySelector(".bar-fill");
    expect(fill).toHaveClass("crit");
  });

  it("zero fill when max is 0", () => {
    const { container } = render(BarMeter, { label: "L", value: 10, max: 0 });
    const fill = container.querySelector<HTMLElement>(".bar-fill");
    expect(fill?.style.width).toBe("0%");
  });

  it("caps fill at 100% when value exceeds max", () => {
    const { container } = render(BarMeter, { label: "L", value: 200, max: 100 });
    const fill = container.querySelector<HTMLElement>(".bar-fill");
    expect(fill?.style.width).toBe("100%");
  });

  it("threshold tick position matches threshold prop", () => {
    const { container } = render(BarMeter, { label: "L", value: 0, max: 100, threshold: 0.6 });
    const tick = container.querySelector<HTMLElement>(".bar-threshold");
    expect(tick?.style.left).toBe("60%");
  });
});
