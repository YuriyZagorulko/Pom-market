import "@testing-library/jest-dom/vitest";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { StrategiesListHeader } from "./strategies-list-header";

describe("StrategiesListHeader", () => {
  it("shows create strategy link", () => {
    render(<StrategiesListHeader />);
    const link = screen.getByRole("link", { name: "Create strategy" });
    expect(link).toHaveAttribute("href", "/strategies/new");
  });
});
