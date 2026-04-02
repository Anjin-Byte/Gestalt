import { render } from "@testing-library/svelte";
import { describe, it, expect } from "vitest";
import BitField from "./BitField.svelte";

describe("BitField", () => {
  it("renders all flag labels as pill text", () => {
    const { getByText } = render(BitField, {
      flags: [
        { label: "ZW", value: true },
        { label: "BFC", value: false },
        { label: "ST", value: undefined },
      ],
    });
    expect(getByText("ZW")).toBeInTheDocument();
    expect(getByText("BFC")).toBeInTheDocument();
    expect(getByText("ST")).toBeInTheDocument();
  });

  it("renders the row label when provided", () => {
    const { getByText } = render(BitField, {
      label: "Pipeline State",
      flags: [{ label: "ZW", value: true }],
    });
    expect(getByText("Pipeline State")).toBeInTheDocument();
  });

  it("no row label when omitted", () => {
    const { container } = render(BitField, {
      flags: [{ label: "ZW", value: true }],
    });
    expect(container.querySelector(".bf-label")).not.toBeInTheDocument();
  });

  it("applies bf-on class for true values", () => {
    const { container } = render(BitField, {
      flags: [{ label: "ZW", value: true }],
    });
    expect(container.querySelector(".bf-on")).toBeInTheDocument();
  });

  it("applies bf-off class for false values", () => {
    const { container } = render(BitField, {
      flags: [{ label: "BFC", value: false }],
    });
    expect(container.querySelector(".bf-off")).toBeInTheDocument();
  });

  it("applies bf-unknown class for undefined values", () => {
    const { container } = render(BitField, {
      flags: [{ label: "ST", value: undefined }],
    });
    expect(container.querySelector(".bf-unknown")).toBeInTheDocument();
  });

  it("renders correct number of pills with correct states", () => {
    const { container } = render(BitField, {
      flags: [
        { label: "A", value: true },
        { label: "B", value: false },
        { label: "C", value: true },
        { label: "D", value: undefined },
        { label: "E", value: false },
      ],
    });
    expect(container.querySelectorAll(".bf-flag")).toHaveLength(5);
    expect(container.querySelectorAll(".bf-on")).toHaveLength(2);
    expect(container.querySelectorAll(".bf-off")).toHaveLength(2);
    expect(container.querySelectorAll(".bf-unknown")).toHaveLength(1);
  });

  it("sets title attribute from flag title", () => {
    const { container } = render(BitField, {
      flags: [{ label: "ZW", value: true, title: "Depth Write Enabled" }],
    });
    expect(container.querySelector(".bf-flag")).toHaveAttribute("title", "Depth Write Enabled");
  });

  it("falls back to label for title when title not provided", () => {
    const { container } = render(BitField, {
      flags: [{ label: "ZW", value: true }],
    });
    expect(container.querySelector(".bf-flag")).toHaveAttribute("title", "ZW");
  });

  it("empty flags array renders no pills", () => {
    const { container } = render(BitField, {
      label: "Empty",
      flags: [],
    });
    expect(container.querySelectorAll(".bf-flag")).toHaveLength(0);
  });

  it("pill text content matches the label", () => {
    const { container } = render(BitField, {
      flags: [{ label: "DP", value: true }],
    });
    const pill = container.querySelector(".bf-flag");
    expect(pill?.textContent).toBe("DP");
  });
});
