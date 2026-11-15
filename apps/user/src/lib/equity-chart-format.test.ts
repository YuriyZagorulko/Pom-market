import { describe, expect, it } from "vitest";
import {
  formatEquityTooltipWhen,
  formatEquityXLabel,
  nearestIndexByFraction,
  niceYTicks,
  pickXTickIndices,
} from "./equity-chart-format";

describe("equity-chart-format", () => {
  it("niceYTicks spans min–max with round steps", () => {
    const ticks = niceYTicks(0, 2500, 6);
    expect(ticks[0]).toBeLessThanOrEqual(0);
    expect(ticks[ticks.length - 1]).toBeGreaterThanOrEqual(2000);
  });

  it("formatEquityXLabel uses time for 1D", () => {
    const s = formatEquityXLabel("2026-11-11T15:30:00.000Z", "1D");
    expect(s).toMatch(/\d/);
    expect(s.length).toBeLessThan(20);
  });

  it("formatEquityXLabel uses month for ALL", () => {
    const s = formatEquityXLabel("2026-11-11T15:30:00.000Z", "ALL");
    expect(s.toLowerCase()).toMatch(/nov|11/);
  });

  it("formatEquityTooltipWhen is locale datetime", () => {
    const s = formatEquityTooltipWhen("2026-11-11T15:30:00.000Z");
    expect(s).toMatch(/2026/);
    expect(s).toMatch(/30|17|15/);
  });

  it("nearestIndexByFraction picks endpoints", () => {
    expect(nearestIndexByFraction(0, 5)).toBe(0);
    expect(nearestIndexByFraction(1, 5)).toBe(4);
    expect(nearestIndexByFraction(0.5, 5)).toBe(2);
  });

  it("pickXTickIndices covers range", () => {
    const idx = pickXTickIndices(20, 5);
    expect(idx[0]).toBe(0);
    expect(idx[idx.length - 1]).toBe(19);
  });
});
