const REASONS: Record<string, { title: string; detail: string }> = {
  strategy_capital_not_initialized: {
    title: "Strategy capital not funded",
    detail:
      "No cash is reserved in this strategy ledger yet. Enable the strategy or allocate paper capital from the trading account.",
  },
  strategy_disabled: {
    title: "Strategy disabled",
    detail: "Turn on the strategy to copy new trades.",
  },
  source_equity_unavailable: {
    title: "Source portfolio equity unknown",
    detail: "Portfolio-relative sizing needs a fresh equity snapshot for the leader wallet.",
  },
  source_position_value_unavailable: {
    title: "Source open-position value unknown",
    detail:
      "Position Value Ratio needs the leader's open-position market value from a recent Data API snapshot (not wallet equity).",
  },
  source_position_value_stale: {
    title: "Source open-position value stale",
    detail: "The leader position value snapshot is too old. Wait for monitoring to refresh or check the worker.",
  },
  source_position_value_zero: {
    title: "Source has no open positions",
    detail: "Open-position value is zero, so portfolio share cannot be calculated for this buy.",
  },
  no_liquidity: {
    title: "No liquidity within allowed price",
    detail:
      "The CLOB has no depth at or inside your copy limit. In PAPER, micro copies still simulate at the leader price when sizing passes; LIVE rejects here.",
  },
  below_minimum: {
    title: "Below minimum copy size",
    detail:
      "The trade could not be sized above zero after PVR/caps, or LIVE rules require at least $1 / market min shares. PAPER does not use the Polymarket $1 floor when strategy min order is $0.",
  },
  slippage_exceeded: {
    title: "Slippage / market gap",
    detail:
      "The best available CLOB price is too far from the leader trade or your limit. Paper copy was not executed.",
  },
};

export function explainDecisionReason(code: string | null | undefined): {
  title?: string;
  detail?: string;
} {
  if (!code) return {};
  return REASONS[code] ?? {};
}
