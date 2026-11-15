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
  equalCopyTradeMultiplier: "1",
  fixedAmountUsd: "25",
  pvrTradeMultiplier: "1",
  copySmallBuys1to1: false,
  smallBuyThresholdUsd: "",
  monitoringEnabled: true,
  accountOperatingMode: "paper_trading",
};

describe("validateCopyStrategyCreateInput", () => {
  it("requires account and trader", () => {
    expect(validateCopyStrategyCreateInput({ ...base, tradingAccountId: "" })).toMatch(/account/i);
    expect(validateCopyStrategyCreateInput({ ...base, sourceTraderId: "" })).toMatch(/trader/i);
  });

  it("requires monitoring and tradable account mode", () => {
    expect(validateCopyStrategyCreateInput({ ...base, monitoringEnabled: false })).toMatch(/monitoring/i);
    expect(validateCopyStrategyCreateInput({ ...base, accountOperatingMode: "monitoring" })).toMatch(
      /Monitoring/i,
    );
    expect(
      validateCopyStrategyCreateInput({ ...base, accountOperatingMode: "live_trading" }),
    ).toBeNull();
  });

  it("validates sizing params per mode", () => {
    expect(
      validateCopyStrategyCreateInput({ ...base, sizingMode: "proportional", equalCopyTradeMultiplier: "0" }),
    ).toMatch(/multiplier/i);
    expect(
      validateCopyStrategyCreateInput({ ...base, sizingMode: "fixed_amount", fixedAmountUsd: "0" }),
    ).toMatch(/Fixed amount/i);
  });

  it("accepts position value ratio without extra fields", () => {
    expect(validateCopyStrategyCreateInput(base)).toBeNull();
  });

  it("requires threshold when small buy 1:1 is enabled", () => {
    expect(
      validateCopyStrategyCreateInput({
        ...base,
        copySmallBuys1to1: true,
        smallBuyThresholdUsd: "0",
      }),
    ).toMatch(/threshold/i);
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

  it("sends small buy settings when enabled on PVR", () => {
    const body = buildCopyStrategyCreateBody({
      ...base,
      copySmallBuys1to1: true,
      smallBuyThresholdUsd: "5",
    });
    expect(body.copy_small_buys_1to1).toBe(true);
    expect(body.small_buy_threshold_usd).toBe(5);
  });

  it("sends small buy settings when enabled on Equal Copy", () => {
    const body = buildCopyStrategyCreateBody({
      ...base,
      sizingMode: "proportional",
      equalCopyTradeMultiplier: "0.5",
      copySmallBuys1to1: true,
      smallBuyThresholdUsd: "5",
    });
    expect(body.proportion).toBe(0.5);
    expect(body.copy_small_buys_1to1).toBe(true);
    expect(body.small_buy_threshold_usd).toBe(5);
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
      buildCopyStrategyCreateBody({ ...base, sizingMode: "proportional", equalCopyTradeMultiplier: "0.5" })
        .proportion,
    ).toBe(0.5);
    expect(
      buildCopyStrategyCreateBody({ ...base, sizingMode: "fixed_amount" }).fixed_amount_usd,
    ).toBe(25);
  });
});
