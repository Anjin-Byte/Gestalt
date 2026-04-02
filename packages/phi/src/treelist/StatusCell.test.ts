import { render } from "@testing-library/svelte";
import { describe, it, expect } from "vitest";
import StatusCell from "./StatusCell.svelte";

describe("StatusCell", () => {
  it("renders the inner StatusIndicator", () => {
    const { container } = render(StatusCell, { status: "ok" });
    expect(container.querySelector(".si-dot")).toBeInTheDocument();
    expect(container.querySelector(".si-ok")).toBeInTheDocument();
  });

  it("centers content inside a .status-cell wrapper", () => {
    const { container } = render(StatusCell, {});
    const cell = container.querySelector(".status-cell");
    expect(cell).toBeInTheDocument();
  });

  it("defaults to idle status", () => {
    const { container } = render(StatusCell, {});
    expect(container.querySelector(".si-idle")).toBeInTheDocument();
  });

  it("passes status prop through to StatusIndicator", () => {
    const { container } = render(StatusCell, { status: "error" });
    expect(container.querySelector(".si-error")).toBeInTheDocument();
  });

  it("passes label prop through to StatusIndicator", () => {
    const { getByText } = render(StatusCell, { status: "warning", label: "Stale" });
    expect(getByText("Stale")).toBeInTheDocument();
  });
});
