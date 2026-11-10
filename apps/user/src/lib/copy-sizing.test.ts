import { describe, expect, it } from "vitest";
import { percentToProportion, proportionToPercent } from "./copy-sizing";

describe("copy-sizing", () => {
  it("converts percent to proportion", () => {
    expect(percentToProportion(10)).toBe(0.1);
    expect(percentToProportion(100)).toBe(1);
    expect(percentToProportion(0)).toBeNull();
    expect(percentToProportion(101)).toBeNull();
  });

  it("converts proportion to percent", () => {
    expect(proportionToPercent(0.1)).toBe(10);
    expect(proportionToPercent(0.255)).toBe(25.5);
  });
});
