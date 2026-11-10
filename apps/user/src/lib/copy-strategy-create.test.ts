import { describe, expect, it } from "vitest";
import {
  buildCopyStrategyCreateBody,
  validateCopyStrategyCreateInput,
  type CopyStrategyCreateInput,
} from "./copy-strategy-create";

const base: CopyStrategyCreateInput = {
  tradingAccountId: "acc-1",
  sourceTraderId: "trader-1",
  allocationMode: "fixed_usd",
  allocationPercent: "25",
  sizingMode: "portfolio_ratio",
  strategyBudgetUsd: "2000",
  maxPositionUsd: "200",
  maxExposureUsd: "2000",
  maxOpenPositions: "10",
  maxSlippageBps: "150",
  minOrderUsd: "1",
  cooldownSeconds: "0",
  copyBuy: true,
  copySell: true,
  copyPercent: "10",
  fixedAmountUsd: "25",
  multiplier: "1.5",
  monitoringEnabled: true,
  accountOperatingMode: "paper_trading",
};

describe("validateCopyStrategyCreateInput", () => {
  it("requires account and trader", () => {
    expect(validateCopyStrategyCreateInput({ ...base, tradingAccountId: "" })).toMatch(/account/i);
    expect(validateCopyStrategyCreateInput({ ...base, sourceTraderId: "" })).toMatch(/trader/i);
  });

  it("requires monitoring and paper mode", () => {
    expect(validateCopyStrategyCreateInput({ ...base, monitoringEnabled: false })).toMatch(/monitoring/i);
    expect(validateCopyStrategyCreateInput({ ...base, accountOperatingMode: "live_trading" })).toMatch(/Paper/i);
  });

  it("validates sizing params per mode", () => {
    expect(
      validateCopyStrategyCreateInput({ ...base, sizingMode: "proportional", copyPercent: "0" }),
    ).toMatch(/percentage/i);
    expect(
      validateCopyStrategyCreateInput({ ...base, sizingMode: "fixed_amount", fixedAmountUsd: "0" }),
    ).toMatch(/Fixed amount/i);
    expect(
      validateCopyStrategyCreateInput({ ...base, sizingMode: "multiplier", multiplier: "-1" }),
    ).toMatch(/Multiplier/i);
  });

  it("accepts valid portfolio-relative input", () => {
    expect(validateCopyStrategyCreateInput(base)).toBeNull();
  });
});

describe("buildCopyStrategyCreateBody", () => {
  it("maps portfolio-relative default body", () => {
    const body = buildCopyStrategyCreateBody(base);
    expect(body).toMatchObject({
      trading_account_id: "acc-1",
      source_trader_id: "trader-1",
      sizing_mode: "portfolio_ratio",
      allocation_mode: "fixed_usd",
      strategy_budget_usd: 2000,
      max_open_positions: 10,
      max_slippage_bps: 150,
      cooldown_seconds: 0,
    });
    expect(body).not.toHaveProperty("enabled");
  });

  it("maps proportional, fixed, and multiplier sizing", () => {
    expect(
      buildCopyStrategyCreateBody({ ...base, sizingMode: "proportional" }).copy_percent,
    ).toBe(10);
    expect(
      buildCopyStrategyCreateBody({ ...base, sizingMode: "fixed_amount" }).fixed_amount_usd,
    ).toBe(25);
    expect(buildCopyStrategyCreateBody({ ...base, sizingMode: "multiplier" }).multiplier).toBe(1.5);
  });
});
