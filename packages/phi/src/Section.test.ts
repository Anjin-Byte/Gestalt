import { render, fireEvent, waitFor } from "@testing-library/svelte";
import { describe, it, expect, beforeEach } from "vitest";
import { createRawSnippet } from "svelte";
import Section from "./Section.svelte";

const childSnippet = createRawSnippet(() => ({
  render: () => `<span data-testid="section-child">child content</span>`,
}));

describe("Section", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("renders the title", () => {
    const { getByText } = render(Section, {
      sectionId: "test-sec",
      title: "My Section",
      children: childSnippet,
    });
    expect(getByText("My Section")).toBeInTheDocument();
  });

  it("is expanded by default", () => {
    const { getByTestId } = render(Section, {
      sectionId: "test-sec",
      title: "S",
      children: childSnippet,
    });
    expect(getByTestId("section-child")).toBeInTheDocument();
  });

  it("collapses when the trigger is clicked", async () => {
    const { getByRole, container } = render(Section, {
      sectionId: "test-sec",
      title: "S",
      children: childSnippet,
    });
    await fireEvent.click(getByRole("button"));
    // Svelte marks the section body as `inert` when collapsed — content becomes
    // non-interactive and hidden from the accessibility tree.
    expect(container.querySelector(".section-body")).toHaveAttribute("inert");
  });

  it("expands again on second click", async () => {
    const { getByRole, getByTestId } = render(Section, {
      sectionId: "test-sec",
      title: "S",
      children: childSnippet,
    });
    await fireEvent.click(getByRole("button"));
    await fireEvent.click(getByRole("button"));
    expect(getByTestId("section-child")).toBeInTheDocument();
  });

  it("trigger button reflects aria-expanded state", async () => {
    const { getByRole } = render(Section, {
      sectionId: "test-sec",
      title: "S",
      children: childSnippet,
    });
    const btn = getByRole("button");
    expect(btn).toHaveAttribute("aria-expanded", "true");
    await fireEvent.click(btn);
    expect(btn).toHaveAttribute("aria-expanded", "false");
  });

  it("persists collapsed state to localStorage", async () => {
    const { getByRole } = render(Section, {
      sectionId: "persist-sec",
      title: "S",
      children: childSnippet,
    });
    await fireEvent.click(getByRole("button"));
    expect(localStorage.getItem("panel-section:persist-sec")).toBe("false");
  });

  it("reads initial closed state from localStorage", () => {
    localStorage.setItem("panel-section:pre-closed", "false");
    const { queryByTestId } = render(Section, {
      sectionId: "pre-closed",
      title: "S",
      children: childSnippet,
    });
    expect(queryByTestId("section-child")).not.toBeInTheDocument();
  });

  it("applies .card class when card prop is true", () => {
    const { container } = render(Section, {
      sectionId: "card-sec",
      title: "Card",
      children: childSnippet,
      card: true,
    });
    expect(container.querySelector(".section.card")).toBeInTheDocument();
  });

  it("does not apply .card class by default", () => {
    const { container } = render(Section, {
      sectionId: "flat-sec",
      title: "Flat",
      children: childSnippet,
    });
    expect(container.querySelector(".section.card")).not.toBeInTheDocument();
  });

  it("card variant still collapses and expands", async () => {
    const { getByRole, getByTestId, container } = render(Section, {
      sectionId: "card-collapse",
      title: "C",
      children: childSnippet,
      card: true,
    });
    expect(getByTestId("section-child")).toBeInTheDocument();
    await fireEvent.click(getByRole("button"));
    expect(container.querySelector(".section-body")).toHaveAttribute("inert");
    await fireEvent.click(getByRole("button"));
    expect(getByTestId("section-child")).toBeInTheDocument();
  });
});
