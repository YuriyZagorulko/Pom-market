import { describe, expect, it } from "vitest";
import {
  bpsToPercentInputValue,
  formatMaxSlippagePercent,
  parseMaxSlippagePercentToBps,
} from "./max-slippage";

describe("max slippage percent display", () => {
  it("shows 100 BPS as 1%", () => {
    expect(formatMaxSlippagePercent(100)).toBe("1%");
    expect(bpsToPercentInputValue(100)).toBe("1");
  });

  it("shows 150 BPS as 1.5%", () => {
    expect(formatMaxSlippagePercent(150)).toBe("1.5%");
    expect(bpsToPercentInputValue(150)).toBe("1.5");
  });

  it("parses 1.5% to 150 BPS", () => {
    const r = parseMaxSlippagePercentToBps("1.5");
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.bps).toBe(150);
    const r2 = parseMaxSlippagePercentToBps("1.5%");
    expect(r2.ok).toBe(true);
    if (r2.ok) expect(r2.bps).toBe(150);
  });

  it("keeps 150 BPS when saving 1.5%", () => {
    const r = parseMaxSlippagePercentToBps("1.5");
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.bps).toBe(150);
  });

  it("does not double-convert stored values when re-displayed", () => {
    expect(bpsToPercentInputValue(150)).toBe("1.5");
    const save = parseMaxSlippagePercentToBps(bpsToPercentInputValue(150));
    expect(save.ok).toBe(true);
    if (save.ok) expect(save.bps).toBe(150);
  });
});