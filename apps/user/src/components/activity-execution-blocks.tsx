import { formatUsd } from "@/lib/display-format";
import {
  formatEventTime,
  formatPricePerShare,
  formatShares,
  formatUsdc,
} from "@/lib/source-trade-format";
import { usd } from "@/lib/strategy-analytics-format";
import type { ActivityFillLeg, CopyDecisionActivityRow } from "./copy-decision-activity-card";

export function LeaderExecutionBlock({ row }: { row: CopyDecisionActivityRow }) {
  const side = row.side?.toLowerCase() === "sell" ? "SELL" : "BUY";
  const price = formatPricePerShare(row.source_price ?? undefined);
  const shares = formatShares(row.source_shares ?? undefined);
  const total = formatUsdc(row.source_notional_usd ?? undefined);
  return (
    <div className="activity-exec-block leader">
      <h4 className="activity-exec-title">Leader execution</h4>
      <div className="activity-exec-grid">
        <div>
          <span className="muted">Side</span>
          <strong>{side}</strong>
        </div>
        <div>
          <span className="muted">Price</span>
          <strong>{price ?? "—"}</strong>
        </div>
        <div>
          <span className="muted">Shares</span>
          <strong>{shares ?? "—"}</strong>
        </div>
        <div>
          <span className="muted">Total</span>
          <strong>{total ?? "—"}</strong>
        </div>
      </div>
      {row.source_event_at &&
      row.filled_at &&
      Math.abs(new Date(row.source_event_at).getTime() - new Date(row.filled_at).getTime()) > 60_000 ? (
        <p className="muted" style={{ margin: "6px 0 0", fontSize: 12 }}>
          Leader trade {formatEventTime(row.source_event_at)}
        </p>
      ) : row.source_event_at && !row.filled_at ? (
        <p className="muted" style={{ margin: "6px 0 0", fontSize: 12 }}>
          Leader trade {formatEventTime(row.source_event_at)}
        </p>
      ) : null}
    </div>
  );
}

export function YourExecutionBlock({
  row,
  open,
}: {
  row: CopyDecisionActivityRow;
  open: boolean;
}) {
  const side = row.side?.toLowerCase() === "sell" ? "SELL" : "BUY";
  const partial =
    row.order_status === "partially_filled" ||
    (row.execution_status === "partial" && Boolean(row.filled_at));
  const target = Number(row.order_target_shares ?? 0);
  const filled = Number(row.fill_shares ?? 0);
  const remaining =
    partial && target > filled ? formatShares(String(target - filled)) : null;

  if (!row.filled_at && row.execution_status !== "executed" && row.execution_status !== "partial") {
    return (
      <div className="activity-exec-block yours">
        <h4 className="activity-exec-title">Your execution</h4>
        <p className="muted" style={{ margin: 0 }}>
          Not executed
        </p>
      </div>
    );
  }

  const vwap = row.fill_price != null ? formatPricePerShare(row.fill_price) : null;
  const fillShares = formatShares(row.fill_shares ?? undefined);
  const gross =
    row.fill_notional_usd != null
      ? formatUsd(row.fill_notional_usd)
      : row.fill_shares != null && row.fill_price != null
        ? formatUsd(String(Number(row.fill_shares) * Number(row.fill_price)))
        : null;
  const fee = row.fee_usd != null ? usd(row.fee_usd) : null;
  const grossNum = Number(row.fill_notional_usd ?? 0);
  const feeNum = Number(row.fee_usd ?? 0);
  const net =
    grossNum > 0
      ? formatUsd(String(side === "BUY" ? grossNum + feeNum : Math.max(grossNum - feeNum, 0)))
      : null;

  return (
    <div className="activity-exec-block yours">
      <h4 className="activity-exec-title">Your execution</h4>
      {partial ? (
        <span className="muted" style={{ fontSize: 12 }}>
          Partial
        </span>
      ) : null}
      <div className="activity-exec-grid">
        <div>
          <span className="muted">Side</span>
          <strong>{side}</strong>
        </div>
        <div>
          <span className="muted">Avg price (VWAP)</span>
          <strong>{vwap ?? "—"}</strong>
        </div>
        <div>
          <span className="muted">Filled shares</span>
          <strong>{fillShares ?? "—"}</strong>
        </div>
        <div>
          <span className="muted">Filled amount</span>
          <strong>{gross ?? "—"}</strong>
        </div>
        <div>
          <span className="muted">Fee</span>
          <strong>{fee ?? "—"}</strong>
        </div>
        <div>
          <span className="muted">{side === "BUY" ? "Total debited" : "Net proceeds"}</span>
          <strong>{net ?? "—"}</strong>
        </div>
      </div>
      {remaining ? (
        <p className="muted" style={{ margin: "6px 0 0", fontSize: 12 }}>
          Remaining on order: {remaining} shares
        </p>
      ) : null}
      {row.position_open_shares != null &&
      Number(row.position_open_shares) > 0 &&
      side === "SELL" &&
      row.filled_at ? (
        <p className="muted" style={{ margin: "6px 0 0", fontSize: 12 }}>
          Partial close — {formatShares(row.position_open_shares)} shares still open
        </p>
      ) : null}
      {open && row.fill_legs && row.fill_legs.length > 0 ? (
        <ul style={{ margin: "8px 0 0", paddingLeft: 18, fontSize: 12 }}>
          {row.fill_legs.map((leg: ActivityFillLeg) => (
            <li key={leg.fill_id}>
              {formatEventTime(leg.filled_at)} · {formatShares(leg.shares)} @ {formatPricePerShare(leg.price)} →{" "}
              {formatUsd(leg.notional_usd)}
              {leg.fee_usd ? ` · fee ${usd(leg.fee_usd)}` : ""}
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

export function CopyTargetBlock({ row }: { row: CopyDecisionActivityRow }) {
  const displayTarget =
    row.capped_target_notional_usd ??
    row.raw_target_notional_usd ??
    row.target_notional_usd;
  if (displayTarget == null && !row.sizing_explanation) return null;
  return (
    <div className="activity-exec-block target">
      <h4 className="activity-exec-title">Copy target</h4>
      <strong>{displayTarget != null ? usd(displayTarget) : "—"}</strong>
    </div>
  );
}

export function SizingDetailsBlock({ row }: { row: CopyDecisionActivityRow }) {
  const line = row.sizing_explanation?.trim();
  if (!line) return null;
  return (
    <div className="activity-exec-block sizing">
      <h4 className="activity-exec-title">Sizing details</h4>
      <p className="muted" style={{ margin: 0, fontSize: 12, lineHeight: 1.35 }}>
        {line}
      </p>
    </div>
  );
}

const CLOSE_LABELS: Record<string, string> = {
  leader_sell: "Leader SELL",
  hard_close: "Hard Close",
  resolution_win: "Resolution Win",
  resolution_loss: "Resolution Loss",
  resolution_refund: "Refund",
};

function MarketThumb({ title }: { title: string }) {
  const initial = title.trim().charAt(0).toUpperCase() || "?";
  return (
    <div className="activity-thumb" aria-hidden>
      {initial}
    </div>
  );
}

const CAPITAL_LABELS: Record<string, string> = {
  account_paper_funding: "Account funding",
  strategy_allocation: "Strategy allocation",
  strategy_withdrawal: "Withdrawal",
  reconciliation_adjustment: "Reconciliation",
  order_reserve: "Order reserve",
  reserve_release: "Reserve release",
};

export function CapitalActivityCard({ row }: { row: CopyDecisionActivityRow }) {
  const kind = row.ledger_entry_type ?? row.event_title ?? "capital";
  const label = CAPITAL_LABELS[kind] ?? kind.replaceAll("_", " ");
  const amount = row.fill_notional_usd ?? row.settlement_payout_usd;
  return (
    <li className="activity-card panel side-buy activity-capital">
      <div className="activity-card-head">
        <div className="activity-thumb" aria-hidden>
          $
        </div>
        <div className="activity-card-meta">
          <div className="activity-card-title-row">
            <span className="activity-type">Capital</span>
            <span className="side-pill buy">CAPITAL</span>
            <time className="muted">{formatEventTime(row.decided_at)}</time>
          </div>
          <div className="activity-market-title">{label}</div>
        </div>
      </div>
      <div className="activity-card-body">
        <div>
          <span className="muted">Amount</span>
          <strong>{amount != null ? usd(amount) : "—"}</strong>
        </div>
        <div>
          <span className="muted">Ledger type</span>
          <strong>{kind}</strong>
        </div>
      </div>
    </li>
  );
}

export function ClosedPositionCard({ row }: { row: CopyDecisionActivityRow }) {
  const reason = row.close_reason ? CLOSE_LABELS[row.close_reason] ?? row.close_reason : "Closed";
  const roi =
    row.close_cost_basis_usd != null &&
    Number(row.close_cost_basis_usd) > 0 &&
    row.close_realized_pnl_usd != null
      ? `${((Number(row.close_realized_pnl_usd) / Number(row.close_cost_basis_usd)) * 100).toFixed(2)}%`
      : "—";
  return (
    <li className="activity-card panel side-sell activity-closed">
      <div className="activity-card-head">
        <MarketThumb title={row.market_title?.trim() || "Unknown market"} />
        <div className="activity-card-meta">
          <div className="activity-card-title-row">
            <span className="activity-type">Closed position</span>
            <span className="side-pill sell">CLOSED</span>
            <time className="muted">{formatEventTime(row.decided_at)}</time>
          </div>
          <div className="activity-market-title">{row.market_title?.trim() || "Unknown market"}</div>
        </div>
      </div>
      <ViewClosedBody row={row} reason={reason} roi={roi} />
    </li>
  );
}

function ViewClosedBody({
  row,
  reason,
  roi,
}: {
  row: CopyDecisionActivityRow;
  reason: string;
  roi: string;
}) {
  return (
    <div className="activity-card-body" style={{ gridTemplateColumns: "1fr 1fr" }}>
      <div>
        <span className="muted">Outcome</span>
        <strong>{row.outcome?.trim() || "—"}</strong>
      </div>
      <div>
        <span className="muted">Close reason</span>
        <strong>{reason}</strong>
      </div>
      <div>
        <span className="muted">Shares closed</span>
        <strong>{formatShares(row.close_closed_shares ?? undefined) ?? "—"}</strong>
      </div>
      <div>
        <span className="muted">Avg entry</span>
        <strong>{formatPricePerShare(row.close_avg_entry_price ?? undefined) ?? "—"}</strong>
      </div>
      <div>
        <span className="muted">Avg exit</span>
        <strong>
          {row.close_avg_exit_price != null
            ? formatPricePerShare(row.close_avg_exit_price) ?? "—"
            : "N/A"}
        </strong>
      </div>
      <div>
        <span className="muted">Cost basis</span>
        <strong>{row.close_cost_basis_usd != null ? usd(row.close_cost_basis_usd) : "—"}</strong>
      </div>
      <div>
        <span className="muted">Proceeds / payout</span>
        <strong>{row.close_proceeds_usd != null ? usd(row.close_proceeds_usd) : "—"}</strong>
      </div>
      <div>
        <span className="muted">Fees</span>
        <strong>{row.close_fees_usd != null ? usd(row.close_fees_usd) : "—"}</strong>
      </div>
      <div>
        <span className="muted">Realized P&amp;L</span>
        <strong>{row.close_realized_pnl_usd != null ? usd(row.close_realized_pnl_usd) : "—"}</strong>
      </div>
      <div>
        <span className="muted">Realized ROI</span>
        <strong>{roi}</strong>
      </div>
    </div>
  );
}
