"use client";

import { explainDecisionReason } from "@/lib/decision-reason";
import { formatUsd } from "@/lib/display-format";
import {
  formatEventTime,
  formatPricePerShare,
  formatShares,
} from "@/lib/source-trade-format";
import { usd } from "@/lib/strategy-analytics-format";
import {
  formatPaperExecutionDetail,
  paperExecutionShort,
  type PaperExecutionMode,
} from "@/lib/paper-execution";
import {
  CapitalActivityCard,
  ClosedPositionCard,
  CopyTargetBlock,
  LeaderExecutionBlock,
  SizingDetailsBlock,
  YourExecutionBlock,
} from "@/components/activity-execution-blocks";

export type ActivityFillLeg = {
  fill_id: string;
  filled_at: string;
  price: string;
  shares: string;
  notional_usd: string;
  fee_usd?: string | null;
};

export type CopyDecisionActivityRow = {
  decision_id: string;
  decision_status: string;
  execution_status: string;
  activity_kind?: string;
  feed_type?: string;
  ledger_entry_type?: string | null;
  event_title?: string | null;
  settlement_payout_usd?: string | null;
  side: string;
  market_title?: string | null;
  outcome?: string | null;
  decided_at: string;
  source_event_at?: string | null;
  detected_at?: string | null;
  source_notional_usd?: string | null;
  source_price?: string | null;
  source_shares?: string | null;
  target_notional_usd?: string | null;
  pvr_trade_multiplier?: string | null;
  sizing_mode?: string | null;
  raw_target_notional_usd?: string | null;
  capped_target_notional_usd?: string | null;
  sizing_explanation?: string | null;
  order_id?: string | null;
  order_status?: string | null;
  order_target_shares?: string | null;
  submitted_at?: string | null;
  filled_at?: string | null;
  fill_price?: string | null;
  fill_shares?: string | null;
  fill_notional_usd?: string | null;
  fill_legs?: ActivityFillLeg[] | null;
  fee_usd?: string | null;
  slippage_vs_source_bps?: number | null;
  slippage_vs_limit_bps?: number | null;
  reason?: string | null;
  reason_title?: string | null;
  reason_detail?: string | null;
  paper_execution_mode?: PaperExecutionMode;
  paper_execution_context?: Record<string, unknown> | null;
  source_event_id?: string | null;
  position_open_shares?: string | null;
  close_reason?: string | null;
  close_closed_shares?: string | null;
  close_avg_entry_price?: string | null;
  close_avg_exit_price?: string | null;
  close_cost_basis_usd?: string | null;
  close_proceeds_usd?: string | null;
  close_fees_usd?: string | null;
  close_realized_pnl_usd?: string | null;
};

function executionLabel(status: string, row: CopyDecisionActivityRow): string {
  if (row.activity_kind === "settlement") {
    if (row.ledger_entry_type === "resolution_redeem") return "Resolution redeem";
    if (row.ledger_entry_type === "resolution_loss") return "Resolution loss";
    if (row.ledger_entry_type === "resolution_refund") return "Resolution refund";
    if (row.ledger_entry_type === "hard_close_sell") return "Hard close sell";
    return "Settlement";
  }
  switch (status) {
    case "executed":
      return "Executed";
    case "partial":
      return "Partial fill";
    case "pending":
      return "Pending";
    case "failed":
      return "Failed";
    case "no_order":
      return "Approved, no order";
    case "skipped":
      return "Skipped";
    case "settled":
      return "Settled";
    default:
      return status;
  }
}

function MarketThumb({ title }: { title: string }) {
  return (
    <div className="activity-thumb" aria-hidden>
      <span>{title.slice(0, 1).toUpperCase()}</span>
    </div>
  );
}

export function CopyDecisionActivityCard({
  row,
  open,
  onToggle,
}: {
  row: CopyDecisionActivityRow;
  open: boolean;
  onToggle: () => void;
}) {
  if (row.activity_kind === "closed") {
    return <ClosedPositionCard row={row} />;
  }
  if (row.activity_kind === "capital") {
    return <CapitalActivityCard row={row} />;
  }
  const isSettlement = row.activity_kind === "settlement";
  const side = isSettlement
    ? "sell"
    : row.side?.toLowerCase() === "sell"
      ? "sell"
      : "buy";
  const title = row.market_title?.trim() || "Unknown market";
  const reasonExplain = explainDecisionReason(row.reason);
  const whyTitle = row.reason_title ?? reasonExplain.title;
  const whyDetail = row.reason_detail ?? reasonExplain.detail;
  const showWhy = row.execution_status === "skipped" || row.execution_status === "failed";
  const fillTotal =
    row.fill_notional_usd != null
      ? formatUsd(row.fill_notional_usd)
      : row.fill_shares != null && row.fill_price != null
        ? formatUsd(String(Number(row.fill_shares) * Number(row.fill_price)))
        : "—";

  return (
    <li className={`activity-card panel side-${side}`}>
      <div className="activity-card-head">
        <MarketThumb title={title} />
        <div className="activity-card-meta">
          <div className="activity-card-title-row">
            <span className="activity-type">{executionLabel(row.execution_status, row)}</span>
            <span className={`side-pill ${side}`}>
              {isSettlement ? "SETTLEMENT" : side.toUpperCase()}
            </span>
            {row.paper_execution_mode ? (
              <span className="muted">{paperExecutionShort(row.paper_execution_mode)}</span>
            ) : null}
            <time className="muted">{formatEventTime(row.decided_at)}</time>
          </div>
          <div className="activity-market-title">{title}</div>
        </div>
      </div>

      <div className="activity-card-body">
        <div>
          <span className="muted">Outcome</span>
          <strong>{row.outcome?.trim() || "—"}</strong>
        </div>
        {isSettlement ? (
          <div>
            <span className="muted">Payout</span>
            <strong>{row.settlement_payout_usd != null ? usd(row.settlement_payout_usd) : "$0"}</strong>
            {row.fill_shares ? (
              <span className="muted" style={{ fontSize: 12 }}>
                {formatShares(row.fill_shares)} shares
              </span>
            ) : null}
          </div>
        ) : null}
        {!isSettlement ? (
          <div style={{ gridColumn: "1 / -1", display: "grid", gap: 12 }}>
            <LeaderExecutionBlock row={row} />
            <CopyTargetBlock row={row} />
            <YourExecutionBlock row={row} open={open} />
            {showWhy && !row.filled_at && whyTitle ? (
              <p className="muted" style={{ margin: 0, fontSize: 13 }}>
                {whyTitle}
              </p>
            ) : null}
            {row.order_status ? (
              <span className="muted" style={{ fontSize: 12 }}>
                Order: {row.order_status}
              </span>
            ) : null}
          </div>
        ) : null}
      </div>

      {showWhy && whyDetail && !open ? (
        <p className="muted" style={{ margin: 0, fontSize: 13, lineHeight: 1.4 }}>
          {whyDetail}
        </p>
      ) : null}

      <div className="activity-card-foot">
        <span className="muted">
          Decision {row.decision_status}
          {row.reason ? ` · ${row.reason}` : ""}
        </span>
        <button type="button" className="ghost activity-expand" onClick={onToggle}>
          {open ? "Hide" : "More"}
        </button>
      </div>

      {open && (
        <div className="activity-details">
          <SizingDetailsBlock row={row} />
          <DetailRow label="Decision ID" value={row.decision_id} />
          {row.source_event_id ? <DetailRow label="Source event ID" value={row.source_event_id} /> : null}
          {row.order_id ? <DetailRow label="Order ID" value={row.order_id} /> : null}
          <DetailRow label="Decision status" value={row.decision_status} />
          {row.reason ? <DetailRow label="Reason code" value={row.reason} /> : null}
          {whyTitle ? <DetailRow label="Summary" value={whyTitle} /> : null}
          {whyDetail ? <DetailRow label="Detail" value={whyDetail} /> : null}
          {row.source_event_at ? (
            <DetailRow label="Leader trade time" value={formatEventTime(row.source_event_at)} />
          ) : null}
          {row.detected_at ? <DetailRow label="Detected" value={formatEventTime(row.detected_at)} /> : null}
          <DetailRow label="Decided" value={formatEventTime(row.decided_at)} />
          {row.submitted_at ? <DetailRow label="Submitted" value={formatEventTime(row.submitted_at)} /> : null}
          {row.filled_at ? <DetailRow label="Last fill" value={formatEventTime(row.filled_at)} /> : null}
          {row.slippage_vs_source_bps != null ? (
            <DetailRow label="Slippage vs leader" value={`${row.slippage_vs_source_bps} bps`} />
          ) : null}
          {row.slippage_vs_limit_bps != null ? (
            <DetailRow label="Slippage vs limit" value={`${row.slippage_vs_limit_bps} bps`} />
          ) : null}
          {row.paper_execution_mode ? (
            <div className="activity-detail-row" style={{ flexDirection: "column", alignItems: "flex-start" }}>
              <span className="muted">Paper execution</span>
              <ul style={{ margin: "4px 0 0", paddingLeft: 18 }}>
                {formatPaperExecutionDetail(
                  row.paper_execution_mode,
                  row.paper_execution_context as Parameters<typeof formatPaperExecutionDetail>[1],
                ).map((line) => (
                  <li key={line}>{line}</li>
                ))}
              </ul>
            </div>
          ) : null}
        </div>
      )}
    </li>
  );
}

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="activity-detail-row">
      <span className="muted">{label}</span>
      <code>{value}</code>
    </div>
  );
}
