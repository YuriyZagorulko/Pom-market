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
  sizingMode: "position_value_ratio",
  strategyBudgetUsd: "2000",
  maxPositionUsd: "200",
  maxOpenPositions: "10",
  maxSlippageBps: "150",
  minOrderUsd: "1",
  cooldownSeconds: "0",
  copyBuy: true,
  copySell: true,
  allowSubminimumPaperTrades: false,
  copyPercent: "10",
  fixedAmountUsd: "25",
  pvrTradeMultiplier: "1",
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
  });

  it("accepts position value ratio without extra fields", () => {
    expect(validateCopyStrategyCreateInput(base)).toBeNull();
  });
});

describe("buildCopyStrategyCreateBody", () => {
  it("defaults to position_value_ratio", () => {
    const body = buildCopyStrategyCreateBody(base);
    expect(body).toMatchObject({
      trading_account_id: "acc-1",
      source_trader_id: "trader-1",
      sizing_mode: "position_value_ratio",
      allocation_mode: "fixed_usd",
      strategy_budget_usd: 2000,
    });
    expect(body).not.toHaveProperty("enabled");
  });

  it("sends pvr multiplier when not 1", () => {
    expect(buildCopyStrategyCreateBody({ ...base, pvrTradeMultiplier: "10" }).multiplier).toBe(10);
    expect(buildCopyStrategyCreateBody(base).multiplier).toBeUndefined();
  });

  it("includes allow_subminimum when enabled", () => {
    expect(
      buildCopyStrategyCreateBody({ ...base, allowSubminimumPaperTrades: true })
        .allow_subminimum_paper_trades,
    ).toBe(true);
  });

  it("maps proportional and fixed sizing", () => {
    expect(
      buildCopyStrategyCreateBody({ ...base, sizingMode: "proportional" }).copy_percent,
    ).toBe(10);
    expect(
      buildCopyStrategyCreateBody({ ...base, sizingMode: "fixed_amount" }).fixed_amount_usd,
    ).toBe(25);
  });
});
