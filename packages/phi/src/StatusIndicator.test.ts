import { render } from "@testing-library/svelte";
import { describe, it, expect } from "vitest";
import StatusIndicator from "./StatusIndicator.svelte";

describe("StatusIndicator", () => {
  it("defaults to idle status", () => {
    const { container } = render(StatusIndicator, {});
    expect(container.querySelector(".si-idle")).toBeInTheDocument();
  });

  it("applies ok class for ok status", () => {
    const { container } = render(StatusIndicator, { status: "ok" });
    expect(container.querySelector(".si-ok")).toBeInTheDocument();
  });

  it("applies warning class for warning status", () => {
    const { container } = render(StatusIndicator, { status: "warning" });
    expect(container.querySelector(".si-warning")).toBeInTheDocument();
  });

  it("applies error class for error status", () => {
    const { container } = render(StatusIndicator, { status: "error" });
    expect(container.querySelector(".si-error")).toBeInTheDocument();
  });

  it("applies idle class for idle status", () => {
    const { container } = render(StatusIndicator, { status: "idle" });
    expect(container.querySelector(".si-idle")).toBeInTheDocument();
  });

  it("renders label text when label prop is provided", () => {
    const { getByText } = render(StatusIndicator, { status: "ok", label: "Connected" });
    expect(getByText("Connected")).toBeInTheDocument();
  });

  it("does not render label element when label is absent", () => {
    const { container } = render(StatusIndicator, { status: "ok" });
    expect(container.querySelector(".si-label")).not.toBeInTheDocument();
  });

  it("pulse class applied by default when status is ok", () => {
    const { container } = render(StatusIndicator, { status: "ok" });
    expect(container.querySelector(".si-pulse")).toBeInTheDocument();
  });

  it("pulse class not applied by default for non-ok statuses", () => {
    const { container } = render(StatusIndicator, { status: "warning" });
    expect(container.querySelector(".si-pulse")).not.toBeInTheDocument();
  });

  it("pulse prop overrides default pulse behaviour", () => {
    const { container } = render(StatusIndicator, { status: "warning", pulse: true });
    expect(container.querySelector(".si-pulse")).toBeInTheDocument();
  });

  it("pulse=false suppresses pulse even for ok status", () => {
    const { container } = render(StatusIndicator, { status: "ok", pulse: false });
    expect(container.querySelector(".si-pulse")).not.toBeInTheDocument();
  });
});
