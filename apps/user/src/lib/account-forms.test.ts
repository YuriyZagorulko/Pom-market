import { describe, expect, it } from "vitest";
import {
  accountRiskFormValues,
  buildAccountModeBody,
  buildAccountRenameBody,
  buildAccountRiskPatchBody,
  buildCredentialsRotateBody,
  parseStrictInt,
  parseUsdAmount,
} from "./account-forms";

describe("account risk serialization (regression)", () => {
  it('sends max_slippage_bps as JSON number, not string "150"', () => {
    const result = buildAccountRiskPatchBody({
      max_order_usd: "50.00000000",
      max_daily_loss_usd: "100.00000000",
      max_exposure_usd: "500.00000000",
      max_slippage_bps: "150",
      paper_cash_usd: "5000",
    });
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.body.max_slippage_bps).toBe(150);
    expect(typeof result.body.max_slippage_bps).toBe("number");
    const json = JSON.stringify(result.body);
    expect(json).toContain('"max_slippage_bps":150');
    expect(json).not.toMatch(/"max_slippage_bps":"150"/);
    expect(result.body.max_order_usd).toBe("50.00000000");
    expect(result.body.paper_cash_usd).toBe("5000");
  });

  it("rejects fractional slippage bps", () => {
    const result = buildAccountRiskPatchBody({
      max_order_usd: "50",
      max_daily_loss_usd: "100",
      max_exposure_usd: "500",
      max_slippage_bps: "150.5",
      paper_cash_usd: "5000",
    });
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.error).toMatch(/whole number/i);
  });

  it("rejects empty slippage without coercing to 0", () => {
    const r = parseStrictInt("", "Max slippage (bps)");
    expect(r.ok).toBe(false);
    const result = buildAccountRiskPatchBody({
      max_order_usd: "1",
      max_daily_loss_usd: "1",
      max_exposure_usd: "1",
      max_slippage_bps: "",
      paper_cash_usd: "1",
    });
    expect(result.ok).toBe(false);
  });

  it("rejects invalid USD and negative amounts", () => {
    expect(parseUsdAmount("", "X").ok).toBe(false);
    expect(parseUsdAmount("abc", "X").ok).toBe(false);
    expect(parseUsdAmount("-1", "X").ok).toBe(false);
    expect(parseUsdAmount("0", "X").ok).toBe(true);
  });

  it("preserves decimal string precision for money fields", () => {
    const result = buildAccountRiskPatchBody({
      max_order_usd: "50.12345678",
      max_daily_loss_usd: "0.01",
      max_exposure_usd: "999999.99",
      max_slippage_bps: "0",
      paper_cash_usd: "5000.00000000",
    });
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.body.max_order_usd).toBe("50.12345678");
    expect(result.body.paper_cash_usd).toBe("5000.00000000");
  });

  it("accepts boundary slippage 0 and rejects above max", () => {
    expect(parseStrictInt("0", "bps", { max: 50_000 }).ok).toBe(true);
    expect(parseStrictInt("50000", "bps", { max: 50_000 }).ok).toBe(true);
    expect(parseStrictInt("50001", "bps", { max: 50_000 }).ok).toBe(false);
  });
});

describe("accountRiskFormValues", () => {
  it("maps API risk into form strings", () => {
    const v = accountRiskFormValues({
      max_order_usd: "50.00000000",
      max_slippage_bps: 150,
      paper_cash_usd: 5000,
    });
    expect(v.max_slippage_bps).toBe("150");
    expect(v.paper_cash_usd).toBe("5000");
  });
});

describe("account edit — other sections", () => {
  it("buildAccountRenameBody", () => {
    expect(buildAccountRenameBody({ name: "  Desk A  " })).toEqual({ ok: true, body: { name: "Desk A" } });
    expect(buildAccountRenameBody({ name: "" }).ok).toBe(false);
  });

  it("buildAccountModeBody", () => {
    const paper = buildAccountModeBody({ mode: "paper_trading", confirm_live: "off" });
    expect(paper).toEqual({ ok: true, body: { mode: "paper_trading", confirm_live: false } });
    const live = buildAccountModeBody({
      mode: "live_trading",
      confirm_live: "on",
      confirmation: "ENABLE LIVE TRADING",
    });
    expect(live).toMatchObject({
      ok: true,
      body: { mode: "live_trading", confirm_live: true, confirmation: "ENABLE LIVE TRADING" },
    });
  });

  it("buildCredentialsRotateBody omits empty fields", () => {
    const body = buildCredentialsRotateBody(
      {
        signature_type: "eoa",
        signer_address: "0xabc",
        funder_address: "",
        private_key: "0xkey",
      },
      "My account",
    );
    expect(body).toEqual({
      name: "My account",
      signature_type: "eoa",
      signer_address: "0xabc",
      private_key: "0xkey",
    });
    expect(body).not.toHaveProperty("funder_address");
  });
});
