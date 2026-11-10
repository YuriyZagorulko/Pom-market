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
};

export function explainDecisionReason(code: string | null | undefined): {
  title?: string;
  detail?: string;
} {
  if (!code) return {};
  return REASONS[code] ?? {};
}
