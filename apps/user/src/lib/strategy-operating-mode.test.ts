import { describe, expect, it } from "vitest";
import {
  ACCOUNT_CREDENTIALS_REQUIRED_CODE,
  buildStrategyOperatingModeBody,
  formatTradingModeApiError,
  LIVE_MODE_PHRASE,
} from "./strategy-operating-mode";

describe("buildStrategyOperatingModeBody", () => {
  it("builds paper body", () => {
    expect(buildStrategyOperatingModeBody({ operating_mode: "paper_trading", confirm_live: false })).toEqual({
      ok: true,
      body: { operating_mode: "paper_trading", confirm_live: false },
    });
  });

  it("requires phrase for live", () => {
    const r = buildStrategyOperatingModeBody({
      operating_mode: "live_trading",
      confirm_live: true,
      confirmation: "wrong",
    });
    expect(r.ok).toBe(false);
    const ok = buildStrategyOperatingModeBody({
      operating_mode: "live_trading",
      confirm_live: true,
      confirmation: LIVE_MODE_PHRASE,
    });
    expect(ok).toEqual({
      ok: true,
      body: {
        operating_mode: "live_trading",
        confirm_live: true,
        confirmation: LIVE_MODE_PHRASE,
      },
    });
  });
});

describe("formatTradingModeApiError", () => {
  it("maps credentials code to actionable text", () => {
    expect(formatTradingModeApiError(ACCOUNT_CREDENTIALS_REQUIRED_CODE)).toMatch(/private key/i);
  });
});
