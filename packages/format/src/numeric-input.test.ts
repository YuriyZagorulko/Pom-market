import { describe, expect, it } from "vitest";
import { formatNumericInputDisplay, isIntermediateNumericInput } from "./numeric-input";

describe("formatNumericInputDisplay", () => {
  it("formats fixed-decimal API strings", () => {
    expect(formatNumericInputDisplay("10.00000000")).toBe("10");
    expect(formatNumericInputDisplay("2000.00000000")).toBe("2000");
    expect(formatNumericInputDisplay("1.00000000")).toBe("1");
    expect(formatNumericInputDisplay("10.50000000")).toBe("10.5");
    expect(formatNumericInputDisplay("0.25000000")).toBe("0.25");
    expect(formatNumericInputDisplay("0.00010000")).toBe("0.0001");
  });

  it("handles numbers and negatives", () => {
    expect(formatNumericInputDisplay(42)).toBe("42");
    expect(formatNumericInputDisplay("-3.50000000")).toBe("-3.5");
  });

  it("handles empty", () => {
    expect(formatNumericInputDisplay(null)).toBe("");
    expect(formatNumericInputDisplay(undefined)).toBe("");
    expect(formatNumericInputDisplay("")).toBe("");
  });

  it("preserves high-precision tails", () => {
    expect(formatNumericInputDisplay("1.23456789")).toBe("1.23456789");
  });
});

describe("isIntermediateNumericInput", () => {
  it("allows partial typing", () => {
    expect(isIntermediateNumericInput("10.")).toBe(true);
    expect(isIntermediateNumericInput("0.0")).toBe(true);
    expect(isIntermediateNumericInput("")).toBe(true);
  });
});
