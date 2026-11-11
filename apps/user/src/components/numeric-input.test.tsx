import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { NumericInput } from "./numeric-input";

describe("NumericInput", () => {
  it("formats default from API decimal string", () => {
    const { container } = render(<NumericInput name="budget" defaultValue="2000.00000000" />);
    const input = container.querySelector('input[name="budget"]') as HTMLInputElement;
    expect(input.value).toBe("2000");
  });
});
