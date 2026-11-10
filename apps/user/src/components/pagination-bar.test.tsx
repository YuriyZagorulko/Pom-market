import "@testing-library/jest-dom/vitest";
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { PaginationBar } from "./pagination-bar";

describe("PaginationBar", () => {
  it("disables prev on first page", () => {
    render(
      <PaginationBar
        total={100}
        limit={25}
        offset={0}
        page={1}
        pageSizeOptions={[25, 50]}
        onPage={vi.fn()}
        onLimit={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: "Previous" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Next" })).not.toBeDisabled();
  });

  it("shows no results range", () => {
    render(
      <PaginationBar
        total={0}
        limit={25}
        offset={0}
        page={1}
        pageSizeOptions={[25]}
        onPage={vi.fn()}
        onLimit={vi.fn()}
      />,
    );
    expect(screen.getByText("No results")).toBeTruthy();
    expect(screen.getByText(/Page 1 of 1/)).toBeTruthy();
  });
});
