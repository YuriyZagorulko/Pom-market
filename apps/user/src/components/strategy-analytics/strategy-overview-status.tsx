import { usd } from "@/lib/strategy-analytics-format";
import {
  capitalFundingBadgeClass,
  capitalFundingLabel,
  ENABLED_UNFUNDED_WARNING,
  type CapitalFundingStatus,
} from "@/lib/strategy-capital-status";

type CapitalMetrics = {
  configured_allocation_cap_usd: string;
  funded_capital_usd: string;
  available_capital_usd: string;
  capital_funding_status: CapitalFundingStatus;
};

export function StrategyStatusBadges({
  enabled,
  capitalFundingStatus,
}: {
  enabled: boolean;
  capitalFundingStatus?: CapitalFundingStatus;
}) {
  return (
    <div className="strategy-status-badges" style={{ display: "flex", flexWrap: "wrap", gap: 8, alignItems: "center" }}>
      <span className={enabled ? "status-badge strategy-enabled" : "status-badge strategy-disabled"}>
        {enabled ? "Enabled" : "Disabled"}
      </span>
      {capitalFundingStatus && (
        <span className={capitalFundingBadgeClass(capitalFundingStatus)}>
          {capitalFundingLabel(capitalFundingStatus)}
        </span>
      )}
    </div>
  );
}

export function StrategyCapitalOverview({
  enabled,
  metrics,
  onFund,
  funding,
}: {
  enabled: boolean;
  metrics: CapitalMetrics;
  onFund?: () => void;
  funding?: boolean;
}) {
  return (
    <div className="panel strategy-capital-overview" style={{ display: "grid", gap: 12 }}>
      <div className="row" style={{ alignItems: "flex-start" }}>
        <div>
          <h2 style={{ margin: 0, fontSize: 18 }}>Capital</h2>
          <p className="muted" style={{ margin: "4px 0 0", fontSize: 13 }}>
            Configured allocation is your budget setting; funded and available amounts come from the strategy ledger only.
          </p>
        </div>
        <span className={capitalFundingBadgeClass(metrics.capital_funding_status)}>
          {capitalFundingLabel(metrics.capital_funding_status)}
        </span>
      </div>
      <div className="metric-grid" style={{ margin: 0 }}>
        <div className="panel metric-card">
          <span className="muted" style={{ fontSize: 12 }}>
            Configured allocation
          </span>
          <strong>{usd(metrics.configured_allocation_cap_usd)}</strong>
        </div>
        <div className="panel metric-card">
          <span className="muted" style={{ fontSize: 12 }}>
            Funded amount
          </span>
          <strong>{usd(metrics.funded_capital_usd)}</strong>
        </div>
        <div className="panel metric-card">
          <span className="muted" style={{ fontSize: 12 }}>
            Available cash
          </span>
          <strong>{usd(metrics.available_capital_usd)}</strong>
        </div>
      </div>
      {enabled && metrics.capital_funding_status === "not_funded" && (
        <p className="warn" style={{ margin: 0, fontSize: 14 }} role="alert">
          {ENABLED_UNFUNDED_WARNING}
        </p>
      )}
      {enabled &&
        (metrics.capital_funding_status === "not_funded" ||
          metrics.capital_funding_status === "partially_funded") &&
        onFund && (
          <div className="row" style={{ justifyContent: "flex-start" }}>
            <button type="button" disabled={funding} onClick={onFund}>
              {funding ? "Funding…" : "Fund from account cash"}
            </button>
            <span className="muted" style={{ fontSize: 13 }}>
              Transfers unallocated PAPER cash into this strategy ledger (Save / Enable also trigger this when the API is reachable).
            </span>
          </div>
        )}
    </div>
  );
}
