"use client";

import { useMemo } from "react";
import { usd } from "@/lib/strategy-analytics-format";

export type EquityPoint = {
  at: string;
  equity_metric_usd: string;
  total_pnl_usd: string;
  data_quality?: string;
};

type Props = {
  points: EquityPoint[];
  loading?: boolean;
  error?: string;
  emptyNote?: string;
  equityLabel?: string;
  equityNote?: string;
};

export function EquityChart({ points, loading, error, emptyNote, equityLabel, equityNote }: Props) {
  const poly = useMemo(() => {
    if (points.length < 2) return null;
    const vals = points.map((p) => Number(p.equity_metric_usd));
    const min = Math.min(...vals);
    const max = Math.max(...vals);
    const span = max - min || 1;
    const w = 640;
    const h = 200;
    const pad = 8;
    return points
      .map((p, i) => {
        const x = pad + (i / (points.length - 1)) * (w - pad * 2);
        const y = h - pad - ((Number(p.equity_metric_usd) - min) / span) * (h - pad * 2);
        return `${x},${y}`;
      })
      .join(" ");
  }, [points]);

  if (loading) return <p className="muted">Loading chart…</p>;
  if (error) return <p className="error">{error}</p>;
  if (!points.length) {
    return (
      <div className="panel" style={{ padding: 16 }}>
        <p className="muted" style={{ margin: 0 }}>
          {emptyNote || "No historical points yet."}
        </p>
      </div>
    );
  }

  const last = points[points.length - 1];

  return (
    <div className="panel" style={{ display: "grid", gap: 8 }}>
      <div className="row" style={{ alignItems: "baseline" }}>
        <h2 style={{ margin: 0 }}>{equityLabel ?? "Partial equity"}</h2>
        <strong>{usd(last.equity_metric_usd)}</strong>
      </div>
      {equityNote && (
        <p className="muted" style={{ margin: 0, fontSize: 13 }}>
          {equityNote}
        </p>
      )}
      {poly ? (
        <svg viewBox="0 0 640 200" role="img" aria-label="Equity over time" style={{ width: "100%", height: 200 }}>
          <polyline fill="none" stroke="#22c55e" strokeWidth="2" points={poly} />
        </svg>
      ) : (
        <p className="muted">Need at least two snapshots to draw a trend.</p>
      )}
      <p className="muted" style={{ margin: 0, fontSize: 12 }}>
        Latest P&L: {usd(last.total_pnl_usd)} · {new Date(last.at).toLocaleString()}
      </p>
    </div>
  );
}
