"use client";

import { explainDecisionReason } from "@/lib/decision-reason";
import {
  formatEventTime,
  formatPricePerShare,
  formatShares,
  formatUsdc,
} from "@/lib/source-trade-format";
import { usd } from "@/lib/strategy-analytics-format";
import {
  formatPaperExecutionDetail,
  paperExecutionShort,
  type PaperExecutionMode,
} from "@/lib/paper-execution";

export type CopyDecisionActivityRow = {
  decision_id: string;
  decision_status: string;
  execution_status: string;
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
  order_id?: string | null;
  order_status?: string | null;
  submitted_at?: string | null;
  filled_at?: string | null;
  fill_price?: string | null;
  fill_shares?: string | null;
  fee_usd?: string | null;
  slippage_vs_source_bps?: number | null;
  slippage_vs_limit_bps?: number | null;
  reason?: string | null;
  reason_title?: string | null;
  reason_detail?: string | null;
  paper_execution_mode?: PaperExecutionMode;
  paper_execution_context?: Record<string, unknown> | null;
  source_event_id?: string | null;
};

function executionLabel(status: string): string {
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
  const side = row.side?.toLowerCase() === "sell" ? "sell" : "buy";
  const title = row.market_title?.trim() || "Unknown market";
  const reasonExplain = explainDecisionReason(row.reason);
  const whyTitle = row.reason_title ?? reasonExplain.title;
  const whyDetail = row.reason_detail ?? reasonExplain.detail;
  const showWhy = row.execution_status === "skipped" || row.execution_status === "failed";
  const leaderShares = formatShares(row.source_shares ?? undefined);
  const leaderPrice = formatPricePerShare(row.source_price ?? undefined);
  const leaderTotal = formatUsdc(row.source_notional_usd ?? undefined);
  const targetTotal = row.target_notional_usd != null ? usd(row.target_notional_usd) : "—";
  const pvrMult =
    row.pvr_trade_multiplier != null && Number(row.pvr_trade_multiplier) > 1
      ? Number(row.pvr_trade_multiplier)
      : null;
  const leaderUsd = row.source_notional_usd != null ? Number(row.source_notional_usd) : null;
  const targetUsd = row.target_notional_usd != null ? Number(row.target_notional_usd) : null;
  const impliedRatio =
    pvrMult == null && leaderUsd != null && leaderUsd > 0 && targetUsd != null && targetUsd > 0
      ? targetUsd / leaderUsd
      : null;
  const fillShares = formatShares(row.fill_shares ?? undefined);
  const fillPrice = row.fill_price != null ? usd(row.fill_price, 4) : "—";
  const fillTotal =
    row.fill_shares != null && row.fill_price != null
      ? usd(String(Number(row.fill_shares) * Number(row.fill_price)))
      : "—";

  return (
    <li className={`activity-card panel side-${side}`}>
      <div className="activity-card-head">
        <MarketThumb title={title} />
        <div className="activity-card-meta">
          <div className="activity-card-title-row">
            <span className="activity-type">{executionLabel(row.execution_status)}</span>
            <span className={`side-pill ${side}`}>{side.toUpperCase()}</span>
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
        <div>
          <span className="muted">Leader</span>
          <strong>
            {leaderShares ?? "—"} @ {leaderPrice ?? "—"}
          </strong>
          <span className="muted" style={{ fontSize: 12 }}>
            {leaderTotal ?? "—"} total
          </span>
        </div>
        <div>
          <span className="muted">Copy target</span>
          <strong>{targetTotal}</strong>
          {pvrMult != null ? (
            <span className="muted" style={{ fontSize: 12 }}>
              {leaderUsd != null
                ? `Leader ${formatUsdc(row.source_notional_usd ?? undefined) ?? "—"} × ${pvrMult}`
                : `× ${pvrMult} PVR`}
              {targetUsd != null ? ` → ${targetTotal}` : ""}
            </span>
          ) : row.sizing_mode === "multiplier" && impliedRatio != null && impliedRatio > 1.001 ? (
            <span className="muted" style={{ fontSize: 12 }}>
              Leader {formatUsdc(row.source_notional_usd ?? undefined) ?? "—"} × {impliedRatio.toFixed(2)}
            </span>
          ) : null}
          {row.order_status ? (
            <span className="muted" style={{ fontSize: 12 }}>
              Order: {row.order_status}
            </span>
          ) : null}
        </div>
        <div>
          <span className="muted">{row.filled_at ? "Your fill" : "Result"}</span>
          <strong>
            {row.filled_at ? (
              <>
                {fillShares ?? "—"} @ {fillPrice}
              </>
            ) : showWhy && whyTitle ? (
              whyTitle
            ) : (
              executionLabel(row.execution_status)
            )}
          </strong>
          <span className="muted" style={{ fontSize: 12 }}>
            {row.filled_at
              ? `${fillTotal}${row.fee_usd ? ` · fee ${usd(row.fee_usd)}` : ""}`
              : row.reason
                ? row.reason.replaceAll("_", " ")
                : "—"}
          </span>
        </div>
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
          <DetailRow label="Decision ID" value={row.decision_id} />
          {row.source_event_id ? <DetailRow label="Source event ID" value={row.source_event_id} /> : null}
          {row.order_id ? <DetailRow label="Order ID" value={row.order_id} /> : null}
          <DetailRow label="Decision status" value={row.decision_status} />
          {row.reason ? <DetailRow label="Reason code" value={row.reason} /> : null}
          {whyTitle ? <DetailRow label="Summary" value={whyTitle} /> : null}
          {whyDetail ? <DetailRow label="Detail" value={whyDetail} /> : null}
          {row.source_event_at ? <DetailRow label="Leader trade time" value={row.source_event_at} /> : null}
          {row.detected_at ? <DetailRow label="Detected" value={row.detected_at} /> : null}
          <DetailRow label="Decided" value={row.decided_at} />
          {row.submitted_at ? <DetailRow label="Submitted" value={row.submitted_at} /> : null}
          {row.filled_at ? <DetailRow label="Filled" value={row.filled_at} /> : null}
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
