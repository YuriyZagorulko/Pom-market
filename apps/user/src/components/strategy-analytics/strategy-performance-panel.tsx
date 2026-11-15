"use client";

import { Fragment, useMemo, useState } from "react";
import { PaginationBar } from "@/components/pagination-bar";
import { useResource } from "@/components/desk";
import { pnlClass, pct, usd } from "@/lib/strategy-analytics-format";
import { EquityChart, type EquityPoint } from "./equity-chart";
import { MetricCard } from "./strategy-metric-card";

type PerfSection = "summary" | "positions" | "events" | "execution";

type PositionRow = {
  id: string;
  token_id: string;
  market_title?: string | null;
  outcome?: string | null;
  status: string;
  open_shares: string;
  cost_basis_usd: string;
  realized_pnl_usd: string;
  unrealized_pnl_usd: string;
  return_percent?: string | null;
  mark_source?: string;
  closed_at?: string | null;
  proceeds_usd?: string | null;
};

type Dashboard = {
  metrics: {
    total_pnl_usd: string;
    realized_pnl_usd: string;
    unrealized_pnl_usd: string;
    roi_percent?: string | null;
    invested_capital_usd: string;
    available_capital_usd: string;
    reserved_capital_usd: string;
    open_positions_count: number;
    closed_positions_count: number;
    total_trades_count: number;
    mark_quality_note?: string | null;
    equity: { note: string; equity_label: string };
  };
  performance: {
    win_rate_percent?: string | null;
    winning_positions: number;
    losing_positions: number;
    avg_win_usd?: string | null;
    avg_loss_usd?: string | null;
    profit_factor?: string | null;
    max_drawdown_percent?: string | null;
    fees_usd: string;
    slippage_bps_avg_clob?: string | null;
    slippage_bps_avg_synthetic?: string | null;
    clob_fill_count: number;
    synthetic_fill_count: number;
    legacy_unlabeled_fill_count: number;
    sample_size_note?: string | null;
    performance_fill_scope?: string;
  };
  top_winners: PositionRow[];
  top_losers: PositionRow[];
};

const PERIODS = [
  ["TODAY", "Today"],
  ["1W", "7 days"],
  ["1M", "30 days"],
  ["3M", "90 days"],
  ["ALL", "All time"],
] as const;

function totalPnl(r: PositionRow): number {
  return Number(r.realized_pnl_usd) + Number(r.unrealized_pnl_usd);
}

function positionStatusLabel(status: string): string {
  if (status === "open") return "Open";
  if (status === "closed") return "Closed";
  return status;
}

export function StrategyPerformancePanel({ strategyId }: { strategyId: string }) {
  const [period, setPeriod] = useState("1M");
  const [customFrom, setCustomFrom] = useState("");
  const [customTo, setCustomTo] = useState("");
  const [section, setSection] = useState<PerfSection>("summary");
  const [posStatus, setPosStatus] = useState("all");
  const [posSort, setPosSort] = useState("profit_desc");
  const [posPnl, setPosPnl] = useState("");
  const [posQ, setPosQ] = useState("");
  const [posPage, setPosPage] = useState(1);
  const [expandedToken, setExpandedToken] = useState<string | null>(null);
  const posLimit = 25;

  const periodQs = useMemo(() => {
    if (customFrom && customTo) {
      return `from=${encodeURIComponent(`${customFrom}T00:00:00Z`)}&to=${encodeURIComponent(`${customTo}T23:59:59Z`)}`;
    }
    return `period=${encodeURIComponent(period)}`;
  }, [period, customFrom, customTo]);

  const dash = useResource<Dashboard>(
    `/api/v1/strategies/${strategyId}/analytics/performance?${periodQs}`,
  );
  const equity = useResource<{
    points: EquityPoint[];
    empty_note?: string;
    equity_label?: string;
    equity_note?: string;
  }>(`/api/v1/strategies/${strategyId}/analytics/equity?${periodQs}`);
  const positionsQs = useMemo(() => {
    const p = new URLSearchParams({
      limit: String(posLimit),
      page: String(posPage),
      status: posStatus,
      sort: posSort,
    });
    if (posPnl) p.set("pnl_filter", posPnl);
    if (posQ.trim()) p.set("q", posQ.trim());
    return p.toString();
  }, [posLimit, posPage, posStatus, posSort, posPnl, posQ]);
  const positions = useResource<{ items: PositionRow[]; total: number; truncation_note?: string }>(
    `/api/v1/strategies/${strategyId}/analytics/performance/positions?${positionsQs}`,
    { enabled: section === "positions" },
  );
  const events = useResource<{ items: KeyEvent[] }>(
    `/api/v1/strategies/${strategyId}/analytics/performance/key-events?${periodQs}`,
    { enabled: section === "events" },
  );
  const execution = useResource<ExecutionStats>(
    `/api/v1/strategies/${strategyId}/analytics/performance/execution?${periodQs}`,
    { enabled: section === "execution" },
  );
  const history = useResource<{ items: HistoryRow[] }>(
    expandedToken
      ? `/api/v1/strategies/${strategyId}/analytics/performance/positions/${encodeURIComponent(expandedToken)}/history`
      : "",
    { enabled: Boolean(expandedToken) },
  );

  const m = dash.data?.metrics;
  const p = dash.data?.performance;

  return (
    <div style={{ display: "grid", gap: 16 }}>
      <div>
        <h2 style={{ margin: "0 0 4px" }}>Performance</h2>
        <p className="muted" style={{ margin: 0, fontSize: 14 }}>
          Strategy-scoped P&amp;L from positions, fills, and capital ledger (PAPER). Realized and unrealized are
          reported separately.
        </p>
      </div>

      <div className="filters date-presets row" role="group" aria-label="Performance period">
        {PERIODS.map(([code, label]) => (
          <button
            key={code}
            type="button"
            className={`ghost date-preset${period === code && !customFrom ? " active" : ""}`}
            onClick={() => {
              setCustomFrom("");
              setCustomTo("");
              setPeriod(code);
            }}
          >
            {label}
          </button>
        ))}
        <input type="date" value={customFrom} onChange={(e) => setCustomFrom(e.target.value)} aria-label="From" />
        <input type="date" value={customTo} onChange={(e) => setCustomTo(e.target.value)} aria-label="To" />
      </div>

      <nav className="filters" aria-label="Performance sections">
        {(
          [
            ["summary", "Summary"],
            ["positions", "Positions"],
            ["events", "Key events"],
            ["execution", "Execution"],
          ] as const
        ).map(([key, label]) => (
          <button
            key={key}
            type="button"
            className={section === key ? "" : "ghost"}
            onClick={() => setSection(key)}
          >
            {label}
          </button>
        ))}
      </nav>

      {dash.error && <p className="error">{dash.error}</p>}
      {!dash.data && !dash.error && <p className="muted">Loading performance…</p>}

      {section === "summary" && m && p && (
        <>
          <MotionSection title="Capital & P&L">
            <MetricCard label="Total P&amp;L" value={usd(m.total_pnl_usd)} className={pnlClass(Number(m.total_pnl_usd))} />
            <MetricCard
              label="Realized P&amp;L"
              value={usd(m.realized_pnl_usd)}
              className={pnlClass(Number(m.realized_pnl_usd))}
            />
            <MetricCard
              label="Unrealized P&amp;L"
              value={usd(m.unrealized_pnl_usd)}
              className={pnlClass(Number(m.unrealized_pnl_usd))}
            />
            <MetricCard label="ROI" value={m.roi_percent != null ? pct(m.roi_percent) : "—"} hint="vs invested cost basis" />
            <MetricCard label="Invested" value={usd(m.invested_capital_usd)} />
            <MetricCard label="Available" value={usd(m.available_capital_usd)} />
            <MetricCard label="Reserved" value={usd(m.reserved_capital_usd)} />
          </MotionSection>

          <MotionSection title="Win / loss stats">
            <MetricCard label="Win rate" value={p.win_rate_percent != null ? pct(p.win_rate_percent) : "—"} />
            <MetricCard label="Wins / losses" value={`${p.winning_positions} / ${p.losing_positions}`} />
            <MetricCard label="Avg profit" value={p.avg_win_usd != null ? usd(p.avg_win_usd) : "—"} />
            <MetricCard label="Avg loss" value={p.avg_loss_usd != null ? usd(p.avg_loss_usd) : "—"} />
            <MetricCard label="Total trades (CLOB fills)" value={String(m.total_trades_count)} />
            <MetricCard label="Profit factor" value={p.profit_factor != null ? String(p.profit_factor) : "—"} />
            <MetricCard label="Max drawdown" value={p.max_drawdown_percent != null ? pct(p.max_drawdown_percent) : "—"} />
            <MetricCard label="Fees (CLOB)" value={usd(p.fees_usd)} />
          </MotionSection>

          <EquityChart
            points={equity.data?.points ?? []}
            period={customFrom && customTo ? "custom" : period}
            loading={!equity.data && !equity.error}
            error={equity.error}
            emptyNote={
              equity.data?.empty_note ||
              "Historical curves use worker snapshots every 15 minutes — not reconstructed from current positions."
            }
            equityLabel={equity.data?.equity_label}
            equityNote={m.equity.note}
          />

          <WinnersBlock title="Top winners (by $ P&L)" rows={dash.data?.top_winners ?? []} />
          <WinnersBlock title="Top losers (by $ P&L)" rows={dash.data?.top_losers ?? []} negative />

          {m.mark_quality_note && (
            <p className="warn" style={{ margin: 0, fontSize: 13 }}>
              {m.mark_quality_note}
            </p>
          )}
          {p.sample_size_note && (
            <p className="muted" style={{ margin: 0, fontSize: 13 }}>
              {p.sample_size_note}
            </p>
          )}
          <p className="muted" style={{ margin: 0, fontSize: 13 }}>
            Slippage and fee averages use {p.performance_fill_scope ?? "clob_fills_only"}. CLOB: {p.clob_fill_count} ·
            Synthetic: {p.synthetic_fill_count} · Legacy unlabeled: {p.legacy_unlabeled_fill_count}
          </p>
        </>
      )}

      {section === "positions" && (
        <>
          <div className="filters list-filters">
            <select value={posStatus} onChange={(e) => { setPosStatus(e.target.value); setPosPage(1); }}>
              <option value="all">All</option>
              <option value="open">Open</option>
              <option value="closed">Closed</option>
            </select>
            <select value={posSort} onChange={(e) => setPosSort(e.target.value)}>
              <option value="profit_desc">Most profitable</option>
              <option value="profit_asc">Most losing</option>
              <option value="roi_desc">Highest ROI</option>
              <option value="invested_desc">Largest investment</option>
              <option value="closed_recent">Recently closed</option>
            </select>
            <select value={posPnl} onChange={(e) => { setPosPnl(e.target.value); setPosPage(1); }}>
              <option value="">Any P&amp;L</option>
              <option value="profit">Profitable</option>
              <option value="loss">Losing</option>
            </select>
            <input
              type="search"
              placeholder="Market or outcome…"
              value={posQ}
              onChange={(e) => { setPosQ(e.target.value); setPosPage(1); }}
            />
          </div>
          {positions.error && <p className="error">{positions.error}</p>}
          {positions.data?.truncation_note && <p className="warn">{positions.data.truncation_note}</p>}
          <PositionTable
            rows={positions.data?.items ?? []}
            loading={!positions.data && !positions.error}
            expandedToken={expandedToken}
            onToggle={(token) => setExpandedToken((c) => (c === token ? null : token))}
            history={history.data?.items}
            historyLoading={Boolean(expandedToken) && !history.data && !history.error}
          />
          {positions.data && positions.data.total > 0 && (
            <PaginationBar
              total={positions.data.total}
              limit={posLimit}
              offset={(posPage - 1) * posLimit}
              page={posPage}
              pageSizeOptions={[25, 50]}
              onPage={setPosPage}
              onLimit={() => undefined}
            />
          )}
        </>
      )}

      {section === "events" && (
        <KeyEventsList items={events.data?.items ?? []} loading={!events.data && !events.error} error={events.error} />
      )}

      {section === "execution" && execution.data && (
        <ExecutionPanel data={execution.data} loading={!execution.data && !execution.error} error={execution.error} />
      )}
    </div>
  );
}

function MotionSection({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="panel metric-grid" style={{ gap: 12 }}>
      <h3 style={{ margin: 0, gridColumn: "1 / -1" }}>{title}</h3>
      {children}
    </div>
  );
}

function WinnersBlock({ title, rows, negative }: { title: string; rows: PositionRow[]; negative?: boolean }) {
  if (!rows.length) return <p className="muted panel">{title}: no positions in this bucket.</p>;
  return (
    <MotionSection title={title}>
      <table className="data-table" style={{ gridColumn: "1 / -1" }}>
        <thead>
          <tr>
            <th>Market</th>
            <th>Status</th>
            <th>Invested</th>
            <th>Total P&amp;L</th>
            <th>ROI</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.id}>
              <td>{r.market_title ?? r.token_id}</td>
              <td>{positionStatusLabel(r.status)}</td>
              <td>{usd(r.cost_basis_usd)}</td>
              <td className={pnlClass(totalPnl(r) * (negative ? -1 : 1))}>{usd(totalPnl(r))}</td>
              <td>{r.return_percent != null ? pct(r.return_percent) : "—"}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </MotionSection>
  );
}

function PositionTable({
  rows,
  loading,
  expandedToken,
  onToggle,
  history,
  historyLoading,
}: {
  rows: PositionRow[];
  loading: boolean;
  expandedToken: string | null;
  onToggle: (token: string) => void;
  history?: HistoryRow[];
  historyLoading?: boolean;
}) {
  if (loading) return <p className="muted">Loading positions…</p>;
  if (!rows.length) return <p className="muted panel">No positions match filters.</p>;
  return (
    <table className="data-table panel">
      <thead>
        <tr>
          <th />
          <th>Market</th>
          <th>Outcome</th>
          <th>Status</th>
          <th>Invested</th>
          <th>Proceeds</th>
          <th>Realized</th>
          <th>Unrealized</th>
          <th>Total P&amp;L</th>
          <th>ROI</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <Fragment key={r.id}>
            <tr>
              <td>
                <button type="button" className="ghost" onClick={() => onToggle(r.token_id)}>
                  {expandedToken === r.token_id ? "−" : "+"}
                </button>
              </td>
              <td>{r.market_title ?? "—"}</td>
              <td>{r.outcome ?? "—"}</td>
              <td>{positionStatusLabel(r.status)}</td>
              <td>{usd(r.cost_basis_usd)}</td>
              <td>{r.proceeds_usd != null ? usd(r.proceeds_usd) : "—"}</td>
              <td className={pnlClass(Number(r.realized_pnl_usd))}>{usd(r.realized_pnl_usd)}</td>
              <td className={pnlClass(Number(r.unrealized_pnl_usd))}>{usd(r.unrealized_pnl_usd)}</td>
              <td className={pnlClass(totalPnl(r))}>{usd(totalPnl(r))}</td>
              <td>{r.return_percent != null ? pct(r.return_percent) : "—"}</td>
            </tr>
            {expandedToken === r.token_id && (
              <tr>
                <td colSpan={10}>
                  {historyLoading && <p className="muted">Loading history…</p>}
                  {history && (
                    <ul style={{ margin: 0, paddingLeft: 18, fontSize: 13 }}>
                      {history.map((h, i) => (
                        <li key={i}>
                          {new Date(h.at).toLocaleString()} · {h.kind}
                          {h.side ? ` ${h.side}` : ""}
                          {h.notional_usd != null ? ` · ${usd(h.notional_usd)}` : ""}
                          {h.execution_mode ? ` · ${h.execution_mode}` : ""}
                          {h.note ? ` — ${h.note}` : ""}
                        </li>
                      ))}
                    </ul>
                  )}
                </td>
              </tr>
            )}
          </Fragment>
        ))}
      </tbody>
    </table>
  );
}

type KeyEvent = {
  id: string;
  at: string;
  kind: string;
  title: string;
  detail?: string | null;
  amount_usd: string;
  market_title?: string | null;
};

function KeyEventsList({
  items,
  loading,
  error,
}: {
  items: KeyEvent[];
  loading: boolean;
  error?: string;
}) {
  if (loading) return <p className="muted">Loading key events…</p>;
  if (error) return <p className="error">{error}</p>;
  if (!items.length) {
    return (
      <p className="muted panel">
        No ledger-grade events in this period. Snapshots and settlement flows appear after trades and resolutions.
      </p>
    );
  }
  return (
    <ul className="record-cards">
      {items.map((e) => (
        <li key={e.id} className="record-card panel">
          <time className="muted" style={{ fontSize: 12 }}>
            {new Date(e.at).toLocaleString()}
          </time>
          <strong>{e.title}</strong>
          {e.market_title && <span className="muted"> · {e.market_title}</span>}
          {e.detail && <p className="muted" style={{ margin: "4px 0 0", fontSize: 13 }}>{e.detail}</p>}
        </li>
      ))}
    </ul>
  );
}

type ExecutionStats = {
  period: string;
  source_events_seen: number;
  decisions_total: number;
  decisions_approved: number;
  decisions_skipped: number;
  decisions_rejected: number;
  orders_filled: number;
  orders_partial: number;
  fills_clob: number;
  fills_synthetic: number;
  fills_legacy_unlabeled: number;
  leader_notional_usd: string;
  copy_notional_usd: string;
  avg_copy_notional_usd?: string | null;
  skip_reason_top: [string, number][];
};

function ExecutionPanel({
  data,
  loading,
  error,
}: {
  data?: ExecutionStats;
  loading: boolean;
  error?: string;
}) {
  if (loading) return <p className="muted">Loading execution stats…</p>;
  if (error) return <p className="error">{error}</p>;
  if (!data) return null;
  return (
    <MotionSection title={`Copy execution (${data.period})`}>
      <MetricCard label="Source events" value={String(data.source_events_seen)} />
      <MetricCard label="Decisions" value={String(data.decisions_total)} />
      <MetricCard label="Approved" value={String(data.decisions_approved)} />
      <MetricCard label="Skipped" value={String(data.decisions_skipped)} />
      <MetricCard label="Rejected" value={String(data.decisions_rejected)} />
      <MetricCard label="Orders filled" value={String(data.orders_filled)} />
      <MetricCard label="Partial fills" value={String(data.orders_partial)} />
      <MetricCard label="Leader notional" value={usd(data.leader_notional_usd)} />
      <MetricCard label="Our copy notional" value={usd(data.copy_notional_usd)} />
      <MetricCard
        label="Avg copy size"
        value={data.avg_copy_notional_usd != null ? usd(data.avg_copy_notional_usd) : "—"}
      />
      <MetricCard label="CLOB fills" value={String(data.fills_clob)} />
      <MetricCard label="Synthetic fills" value={String(data.fills_synthetic)} />
      <MetricCard label="Legacy unlabeled" value={String(data.fills_legacy_unlabeled)} />
      {data.skip_reason_top.length > 0 && (
        <div style={{ gridColumn: "1 / -1" }}>
          <h4 style={{ margin: "8px 0" }}>Top skip reasons</h4>
          <ul style={{ margin: 0, paddingLeft: 18 }}>
            {data.skip_reason_top.map(([reason, n]) => (
              <li key={reason}>
                {reason.replaceAll("_", " ")} — {n}
              </li>
            ))}
          </ul>
        </div>
      )}
    </MotionSection>
  );
}

type HistoryRow = {
  at: string;
  kind: string;
  side?: string | null;
  notional_usd?: string | null;
  execution_mode?: string | null;
  note?: string | null;
};
