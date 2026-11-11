import { describe, expect, it } from "vitest";
import { percentToProportion, proportionToPercent } from "./copy-sizing";

describe("copy-sizing coefficients", () => {
  it("maps ordinary percents to proportion", () => {
    expect(percentToProportion(20)).toBe(0.2);
    expect(proportionToPercent(0.2)).toBe(20);
  });

  it("supports legacy multiplier as percent above 100", () => {
    expect(percentToProportion(150)).toBe(1.5);
    expect(proportionToPercent(1.5)).toBe(150);
    expect(percentToProportion(100)).toBe(1);
  });
});
