"use client";

import { useCallback, useId, useMemo, useRef, useState } from "react";
import {
  formatEquityTooltipWhen,
  formatEquityXLabel,
  nearestIndexByFraction,
  niceYTicks,
  pickXTickIndices,
} from "@/lib/equity-chart-format";
import { usd } from "@/lib/strategy-analytics-format";

export type EquityPoint = {
  at: string;
  equity_metric_usd: string;
  total_pnl_usd?: string;
  deployed_capital_usd?: string;
  reserved_capital_usd?: string;
  available_capital_usd?: string;
  realized_pnl_usd?: string;
  unrealized_pnl_usd?: string;
  data_quality?: string;
};

type Props = {
  points: EquityPoint[];
  period?: string;
  loading?: boolean;
  error?: string;
  emptyNote?: string;
  equityLabel?: string;
  equityNote?: string;
};

const W = 720;
const H = 280;
const M = { top: 12, right: 12, bottom: 52, left: 76 };
const PW = W - M.left - M.right;
const PH = H - M.top - M.bottom;

type SeriesDef = {
  key: keyof EquityPoint;
  label: string;
  color: string;
};

const OPTIONAL_SERIES: SeriesDef[] = [
  { key: "available_capital_usd", label: "Available", color: "#2563eb" },
  { key: "deployed_capital_usd", label: "Invested", color: "#9a4318" },
  { key: "reserved_capital_usd", label: "Reserved", color: "#7c3aed" },
];

function seriesValue(p: EquityPoint | undefined, key: keyof EquityPoint): number | null {
  if (!p) return null;
  const raw = p[key];
  if (raw == null || typeof raw !== "string") return null;
  const n = Number(raw);
  return Number.isFinite(n) ? n : null;
}

function hasSeries(points: EquityPoint[], key: keyof EquityPoint): boolean {
  return points.some((p) => seriesValue(p, key) != null);
}

export function EquityChart({
  points,
  period = "1M",
  loading,
  error,
  emptyNote,
  equityLabel,
  equityNote,
}: Props) {
  const svgRef = useRef<SVGSVGElement>(null);
  const [hoverIndex, setHoverIndex] = useState<number | null>(null);
  const titleId = useId();

  const activeSeries = useMemo(() => {
    const primary: SeriesDef = { key: "equity_metric_usd", label: "Total equity", color: "#1f7a4d" };
    const extra = OPTIONAL_SERIES.filter((s) => hasSeries(points, s.key));
    return [primary, ...extra];
  }, [points]);

  const layout = useMemo(() => {
    if (points.length < 2) return null;
    const allVals: number[] = [];
    for (const p of points) {
      for (const s of activeSeries) {
        const v = seriesValue(p, s.key);
        if (v != null) allVals.push(v);
      }
    }
    if (!allVals.length) return null;
    const min = Math.min(...allVals);
    const max = Math.max(...allVals);
    const yTicks = niceYTicks(min, max, 6);
    const yMin = yTicks[0];
    const yMax = yTicks[yTicks.length - 1];
    const ySpan = yMax - yMin || 1;

    const xAt = (i: number) => M.left + (i / (points.length - 1)) * PW;
    const yAt = (v: number) => M.top + PH - ((v - yMin) / ySpan) * PH;

    const lines = activeSeries.map((s) => ({
      ...s,
      poly: points
        .map((p, i) => {
          const v = seriesValue(p, s.key);
          if (v == null) return null;
          return `${xAt(i)},${yAt(v)}`;
        })
        .filter(Boolean)
        .join(" "),
    }));

    const xTickIdx = pickXTickIndices(points.length, 6);

    return { yTicks, yMin, yMax, xAt, yAt, lines, xTickIdx };
  }, [points, activeSeries]);

  const setHoverFromClientX = useCallback(
    (clientX: number) => {
      if (!layout || !svgRef.current || points.length < 2) return;
      const rect = svgRef.current.getBoundingClientRect();
      if (rect.width <= 0) return;
      const rel = (clientX - rect.left) / rect.width;
      const plotX = rel * W;
      const fraction = (plotX - M.left) / PW;
      const idx = nearestIndexByFraction(fraction, points.length);
      if (idx >= 0 && idx < points.length) setHoverIndex(idx);
    },
    [layout, points.length],
  );

  const onPointerMove = (e: React.PointerEvent<SVGSVGElement>) => {
    if (e.pointerType === "touch") return;
    setHoverFromClientX(e.clientX);
  };

  const onPointerLeave = () => setHoverIndex(null);

  const onPointerDown = (e: React.PointerEvent<SVGSVGElement>) => {
    setHoverFromClientX(e.clientX);
    try {
      svgRef.current?.setPointerCapture(e.pointerId);
    } catch {
      /* not supported in some environments */
    }
  };

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
  const hover = hoverIndex != null && hoverIndex < points.length ? points[hoverIndex] : null;

  return (
    <div className="panel equity-chart-wrap" style={{ display: "grid", gap: 8 }}>
      <div className="row" style={{ alignItems: "baseline" }}>
        <h2 style={{ margin: 0 }} id={titleId}>
          {equityLabel ?? "Strategy equity"}
        </h2>
        <strong>{usd(last.equity_metric_usd)}</strong>
      </div>
      {equityNote && (
        <p className="muted" style={{ margin: 0, fontSize: 13 }}>
          {equityNote}
        </p>
      )}
      {activeSeries.length > 1 && (
        <ul className="equity-chart-legend" aria-hidden>
          {activeSeries.map((s) => (
            <li key={s.key}>
              <span className="equity-chart-swatch" style={{ background: s.color }} />
              {s.label}
            </li>
          ))}
        </ul>
      )}
      {layout ? (
        <div className="equity-chart-svg-host" style={{ position: "relative" }}>
          <svg
            ref={svgRef}
            viewBox={`0 0 ${W} ${H}`}
            role="img"
            aria-labelledby={titleId}
            className="equity-chart-svg"
            onPointerMove={onPointerMove}
            onPointerLeave={onPointerLeave}
            onPointerDown={onPointerDown}
          >
            <title>Equity and capital over time</title>
            {/* Y grid + labels */}
            {layout.yTicks.map((tick) => {
              const y = layout.yAt(tick);
              return (
                <g key={tick}>
                  <line
                    x1={M.left}
                    y1={y}
                    x2={M.left + PW}
                    y2={y}
                    className="equity-chart-grid"
                  />
                  <text x={M.left - 8} y={y + 4} textAnchor="end" className="equity-chart-axis-y">
                    {usd(tick, 0)}
                  </text>
                </g>
              );
            })}
            <text
              x={14}
              y={M.top + PH / 2}
              textAnchor="middle"
              transform={`rotate(-90 14 ${M.top + PH / 2})`}
              className="equity-chart-axis-title"
            >
              Amount (USD)
            </text>

            {/* Series lines */}
            {layout.lines.map(
              (line) =>
                line.poly && (
                  <polyline
                    key={line.key}
                    fill="none"
                    stroke={line.color}
                    strokeWidth={line.key === "equity_metric_usd" ? 2.5 : 1.5}
                    strokeLinejoin="round"
                    opacity={line.key === "equity_metric_usd" ? 1 : 0.85}
                    points={line.poly}
                  />
                ),
            )}

            {/* X ticks */}
            {layout.xTickIdx.map((i) => (
              <text
                key={i}
                x={layout.xAt(i)}
                y={H - 20}
                textAnchor="middle"
                className="equity-chart-axis-x"
              >
                {formatEquityXLabel(points[i].at, period)}
              </text>
            ))}
            <text
              x={M.left + PW / 2}
              y={H - 4}
              textAnchor="middle"
              className="equity-chart-axis-title"
            >
              Time
            </text>

            {/* Crosshair + active dots */}
            {hover != null && hoverIndex != null && (
              <g>
                <line
                  x1={layout.xAt(hoverIndex)}
                  y1={M.top}
                  x2={layout.xAt(hoverIndex)}
                  y2={M.top + PH}
                  className="equity-chart-crosshair"
                />
                {activeSeries.map((s) => {
                  const v = seriesValue(hover, s.key);
                  if (v == null) return null;
                  return (
                    <circle
                      key={s.key}
                      cx={layout.xAt(hoverIndex)}
                      cy={layout.yAt(v)}
                      r={5}
                      fill={s.color}
                      stroke="#fffdf8"
                      strokeWidth={2}
                    />
                  );
                })}
              </g>
            )}
          </svg>
          {hover && hoverIndex != null && (
            <MotionEquityTooltip
              point={hover}
              series={activeSeries}
              hostWidth={svgRef.current?.getBoundingClientRect().width ?? W}
              anchorX={
                (layout.xAt(hoverIndex) / W) * (svgRef.current?.getBoundingClientRect().width ?? W)
              }
            />
          )}
        </div>
      ) : (
        <p className="muted">Need at least two snapshots to draw a trend.</p>
      )}
      <p className="muted" style={{ margin: 0, fontSize: 12 }}>
        Values are snapshot metrics from the API (no interpolation). Latest P&amp;L:{" "}
        {usd(last.total_pnl_usd ?? last.equity_metric_usd)} · {new Date(last.at).toLocaleString()}
      </p>
    </div>
  );
}

function MotionEquityTooltip({
  point,
  series,
  hostWidth,
  anchorX,
}: {
  point: EquityPoint;
  series: SeriesDef[];
  hostWidth: number;
  anchorX: number;
}) {
  const pad = 12;
  const maxW = Math.min(280, hostWidth - pad * 2);
  const flip = anchorX > hostWidth * 0.55;
  const style: React.CSSProperties = {
    position: "absolute",
    top: 8,
    maxWidth: maxW,
    ...(flip ? { right: pad } : { left: Math.min(Math.max(anchorX - 100, pad), hostWidth - maxW - pad) }),
  };

  return (
    <div className="equity-chart-tooltip panel" style={style} role="status">
      <strong style={{ display: "block", marginBottom: 6, fontSize: 13 }}>
        {formatEquityTooltipWhen(point.at)}
      </strong>
      <ul style={{ margin: 0, padding: 0, listStyle: "none", fontSize: 13, lineHeight: 1.45 }}>
        {series.map((s) => {
          const v = seriesValue(point, s.key);
          if (v == null) return null;
          return (
            <li key={s.key} style={{ display: "flex", gap: 8, justifyContent: "space-between" }}>
              <span className="muted">{s.label}</span>
              <span>{usd(v)}</span>
            </li>
          );
        })}
        {point.realized_pnl_usd != null && (
          <li style={{ display: "flex", gap: 8, justifyContent: "space-between" }}>
            <span className="muted">Realized P&amp;L</span>
            <span>{usd(point.realized_pnl_usd)}</span>
          </li>
        )}
        {point.unrealized_pnl_usd != null && (
          <li style={{ display: "flex", gap: 8, justifyContent: "space-between" }}>
            <span className="muted">Unrealized P&amp;L</span>
            <span>{usd(point.unrealized_pnl_usd)}</span>
          </li>
        )}
      </ul>
    </div>
  );
}
