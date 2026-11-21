import { describe, expect, it } from "vitest";
import {
  accountRiskFormValues,
  buildAccountRiskPatchBody,
  parseStrictInt,
} from "./account-forms";

describe("account risk slippage percent UI", () => {
  it("displays 150 BPS as 1.5 in the form", () => {
    const v = accountRiskFormValues({
      max_order_usd: 100,
      max_daily_loss_usd: 50,
      max_slippage_bps: 150,
      paper_cash_usd: 1000,
    });
    expect(v.max_slippage_bps).toBe("1.5");
  });

  it("sends max_slippage_bps as JSON number from percent input", () => {
    const result = buildAccountRiskPatchBody({
      max_order_usd: "100",
      max_daily_loss_usd: "50",
      max_slippage_bps: "1.5",
    });
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.body.max_slippage_bps).toBe(150);
    expect(typeof result.body.max_slippage_bps).toBe("number");
    const json = JSON.stringify(result.body);
    expect(json).toContain('"max_slippage_bps":150');
  });

  it("rejects invalid slippage percent", () => {
    const result = buildAccountRiskPatchBody({
      max_order_usd: "100",
      max_daily_loss_usd: "50",
      max_slippage_bps: "",
    });
    expect(result.ok).toBe(false);
  });

  it("legacy bps field tests still validate strict int helper", () => {
    const r = parseStrictInt("150", "test", { min: 0, max: 50_000 });
    expect(r.ok).toBe(true);
  });
});