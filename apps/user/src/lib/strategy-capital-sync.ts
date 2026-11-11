import { usd } from "@/lib/strategy-analytics-format";

export type CapitalSyncOutcome = {
  configured_capital_usd: string;
  funded_capital_usd: string;
  available_cash_usd: string;
  delta_applied_usd: string;
  bootstrap_quality: string;
};

export function capitalSyncSuccessMessage(outcome: CapitalSyncOutcome): string {
  const delta = Number(outcome.delta_applied_usd);
  if (!Number.isFinite(delta) || delta === 0) {
    return `Strategy ledger already funded at ${usd(outcome.funded_capital_usd)} (configured ${usd(outcome.configured_capital_usd)}).`;
  }
  return `Moved ${usd(outcome.delta_applied_usd)} from account cash into the strategy ledger. Funded ${usd(outcome.funded_capital_usd)}, available ${usd(outcome.available_cash_usd)}.`;
}
