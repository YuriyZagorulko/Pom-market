import { percentToProportion } from "@/lib/copy-sizing";

export type SizingMode = "fixed_amount" | "proportional" | "multiplier" | "portfolio_ratio";
export type AllocationMode = "fixed_usd" | "percent_of_account";

export type CopyStrategyCreateInput = {
  tradingAccountId: string;
  sourceTraderId: string;
  allocationMode: AllocationMode;
  allocationPercent: string;
  sizingMode: SizingMode;
  strategyBudgetUsd: string;
  maxPositionUsd: string;
  maxExposureUsd: string;
  maxOpenPositions: string;
  maxSlippageBps: string;
  minOrderUsd: string;
  cooldownSeconds: string;
  copyBuy: boolean;
  copySell: boolean;
  copyPercent: string;
  fixedAmountUsd: string;
  multiplier: string;
  monitoringEnabled: boolean;
  accountOperatingMode: string | undefined;
};

export function validateCopyStrategyCreateInput(input: CopyStrategyCreateInput): string | null {
  if (!input.sourceTraderId) return "Choose a source trader.";
  if (!input.tradingAccountId) return "Choose a trading account.";
  if (!input.monitoringEnabled) {
    return "Turn on monitoring for the source wallet before creating a strategy.";
  }
  if (input.accountOperatingMode && input.accountOperatingMode !== "paper_trading") {
    return "For now, pick an account in Paper trading mode.";
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
    if (percentToProportion(Number(input.copyPercent)) == null) {
      return "Copy percentage must be between 0 and 100.";
    }
  } else if (input.sizingMode === "fixed_amount") {
    const fixed = Number(input.fixedAmountUsd);
    if (!Number.isFinite(fixed) || fixed <= 0) return "Fixed amount must be greater than zero.";
  } else if (input.sizingMode === "multiplier") {
    const mult = Number(input.multiplier);
    if (!Number.isFinite(mult) || mult <= 0) return "Multiplier must be greater than zero.";
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
    max_exposure_usd: num(input.maxExposureUsd || input.strategyBudgetUsd),
    max_open_positions: intField(input.maxOpenPositions),
    max_slippage_bps: intField(input.maxSlippageBps),
    min_order_usd: num(input.minOrderUsd),
    cooldown_seconds: intField(input.cooldownSeconds),
    copy_buy: input.copyBuy,
    copy_sell: input.copySell,
  };
  if (input.allocationMode === "percent_of_account") {
    body.allocation_percent = num(input.allocationPercent);
  }
  if (input.sizingMode === "proportional") {
    body.copy_percent = num(input.copyPercent);
  } else if (input.sizingMode === "fixed_amount") {
    body.fixed_amount_usd = num(input.fixedAmountUsd);
  } else if (input.sizingMode === "multiplier") {
    body.multiplier = num(input.multiplier);
  }
  return body;
}
