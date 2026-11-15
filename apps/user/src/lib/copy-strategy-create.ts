import { type UserSizingMode } from "@/lib/sizing-mode";

export type { UserSizingMode as SizingMode };
export type AllocationMode = "fixed_usd" | "percent_of_account";

export type CopyStrategyCreateInput = {
  tradingAccountId: string;
  sourceTraderId: string;
  allocationMode: AllocationMode;
  allocationPercent: string;
  sizingMode: UserSizingMode;
  strategyBudgetUsd: string;
  maxPositionUsd: string;
  maxOpenPositions: string;
  maxSlippageBps: string;
  minOrderUsd: string;
  cooldownSeconds: string;
  copyBuy: boolean;
  copySell: boolean;
  allowSubminimumPaperTrades: boolean;
  /** Equal Copy trade multiplier coefficient (1 = 100%). */
  equalCopyTradeMultiplier: string;
  fixedAmountUsd: string;
  pvrTradeMultiplier: string;
  copySmallBuys1to1: boolean;
  smallBuyThresholdUsd: string;
  monitoringEnabled: boolean;
  accountOperatingMode: string | undefined;
};

export function validateCopyStrategyCreateInput(input: CopyStrategyCreateInput): string | null {
  if (!input.sourceTraderId) return "Choose a source trader.";
  if (!input.tradingAccountId) return "Choose a trading account.";
  if (!input.monitoringEnabled) {
    return "Turn on monitoring for the source wallet before creating a strategy.";
  }
  if (
    input.accountOperatingMode &&
    input.accountOperatingMode !== "paper_trading" &&
    input.accountOperatingMode !== "live_trading"
  ) {
    return "Pick a trading account that is not in Monitoring mode.";
  }
  if (input.allocationMode === "fixed_usd") {
    const budget = Number(input.strategyBudgetUsd);
    if (!Number.isFinite(budget) || budget <= 0) return "Allocated capital (USD) must be greater than zero.";
  } else {
    const pct = Number(input.allocationPercent);
    if (!Number.isFinite(pct) || pct <= 0 || pct > 100) {
      return "Allocation percent must be between 0 and 100.";
    }
  }
  if (input.sizingMode === "proportional") {
    const mult = Number(input.equalCopyTradeMultiplier);
    if (!Number.isFinite(mult) || mult <= 0) {
      return "Trade multiplier must be greater than zero.";
    }
    if (mult > 10_000) return "Trade multiplier must be at most 10000.";
  } else if (input.sizingMode === "fixed_amount") {
    const fixed = Number(input.fixedAmountUsd);
    if (!Number.isFinite(fixed) || fixed <= 0) return "Fixed amount must be greater than zero.";
  } else if (input.sizingMode === "position_value_ratio") {
    const mult = Number(input.pvrTradeMultiplier);
    if (!Number.isFinite(mult) || mult <= 0) return "PVR trade multiplier must be greater than zero.";
    if (mult > 10_000) return "PVR trade multiplier must be at most 10000.";
  }
  if (input.copySmallBuys1to1) {
    const thresh = Number(input.smallBuyThresholdUsd);
    if (!Number.isFinite(thresh) || thresh <= 0) {
      return "Small BUY threshold (USD) must be greater than zero when Small BUY Override is enabled.";
    }
  }
  return null;
}

function num(s: string): number {
  const n = Number(s);
  return Number.isFinite(n) ? n : 0;
}

function intField(s: string): number {
  const n = parseInt(s, 10);
  return Number.isFinite(n) ? n : 0;
}

/** Build POST `/api/v1/strategies` body. Strategy is always created disabled on the server. */
export function buildCopyStrategyCreateBody(input: CopyStrategyCreateInput): Record<string, unknown> {
  const budget =
    input.allocationMode === "fixed_usd" ? num(input.strategyBudgetUsd) : 0;
  const body: Record<string, unknown> = {
    trading_account_id: input.tradingAccountId,
    source_trader_id: input.sourceTraderId,
    sizing_mode: input.sizingMode,
    allocation_mode: input.allocationMode,
    strategy_budget_usd: budget,
    max_position_usd: num(input.maxPositionUsd),
    max_open_positions: intField(input.maxOpenPositions),
    max_slippage_bps: intField(input.maxSlippageBps),
    min_order_usd: num(input.minOrderUsd),
    cooldown_seconds: intField(input.cooldownSeconds),
    copy_buy: input.copyBuy,
    copy_sell: input.copySell,
    allow_subminimum_paper_trades: input.allowSubminimumPaperTrades,
  };
  if (input.allocationMode === "percent_of_account") {
    body.allocation_percent = num(input.allocationPercent);
  }
  if (input.sizingMode === "proportional") {
    body.proportion = num(input.equalCopyTradeMultiplier);
  } else if (input.sizingMode === "fixed_amount") {
    body.fixed_amount_usd = num(input.fixedAmountUsd);
  } else if (input.sizingMode === "position_value_ratio") {
    const mult = num(input.pvrTradeMultiplier);
    if (mult > 0 && mult !== 1) {
      body.multiplier = mult;
    }
  }
  if (input.copySmallBuys1to1) {
    body.copy_small_buys_1to1 = true;
    body.small_buy_threshold_usd = num(input.smallBuyThresholdUsd);
  }
  return body;
}
