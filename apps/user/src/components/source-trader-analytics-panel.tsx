"use client";

import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import { CopyButton } from "@/components/copy-button";
import { EquityChart, type EquityPoint } from "@/components/strategy-analytics/equity-chart";
import { MetricCard } from "@/components/strategy-analytics/strategy-metric-card";
import { useSession } from "@/lib/session";
import { pnlClass, usd } from "@/lib/strategy-analytics-format";

type Period = "1D" | "7D" | "30D" | "90D" | "ALL" | "custom";

const PERIODS: Array<{ id: Period; label: string }> = [
  { id: "1D", label: "1 Day" },
  { id: "7D", label: "7 Days" },
  { id: "30D", label: "30 Days" },
  { id: "90D", label: "90 Days" },
  { id: "ALL", label: "All Time" },
  { id: "custom", label: "Custom" },
];

type Dashboard = Record<string, unknown>;

function num(v: unknown): number | null {
  if (v == null) return null;
  const n = Number(v);
  return Number.isFinite(n) ? n : null;
}

function polymarketProfileUrl(wallet: string) {
  return `https://polymarket.com/profile/${wallet}`;
}

function marketUrl(slug: string | null | undefined) {
  if (!slug) return null;
  return `https://polymarket.com/event/${slug}`;
}

export function SourceTraderAnalyticsPanel({
  traderId,
  wallet,
  alias,
}: {
  traderId: string;
  wallet: string;
  alias: string;
}) {
  const { client } = useSession();
  const [period, setPeriod] = useState<Period>("30D");
  const [customFrom, setCustomFrom] = useState("");
  const [customTo, setCustomTo] = useState("");
  const [data, setData] = useState<Dashboard | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tradeSide, setTradeSide] = useState<"" | "buy" | "sell">("");
  const [tradeQ, setTradeQ] = useState("");
  const [trades, setTrades] = useState<Array<Record<string, unknown>>>([]);
  const [tradesTotal, setTradesTotal] = useState(0);
  const [tradesOffset, setTradesOffset] = useState(0);

  const querySuffix = useMemo(() => {
    if (period === "custom" && customFrom && customTo) {
      return `&from=${encodeURIComponent(new Date(customFrom).toISOString())}&to=${encodeURIComponent(new Date(customTo).toISOString())}`;
    }
    if (period !== "custom") return `?period=${period}`;
    return "?period=30D";
  }, [period, customFrom, customTo]);

  const loadDashboard = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const path =
        period === "custom" && customFrom && customTo
          ? `/api/v1/source-traders/${traderId}/analytics?period=custom&from=${encodeURIComponent(new Date(customFrom).toISOString())}&to=${encodeURIComponent(new Date(customTo).toISOString())}`
          : `/api/v1/source-traders/${traderId}/analytics?period=${period}`;
      const d = await client.request<Dashboard>(path);
      setData(d);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load analytics");
    } finally {
      setLoading(false);
    }
  }, [client, traderId, period, customFrom, customTo]);

  const loadTrades = useCallback(
    async (offset = 0) => {
      const params = new URLSearchParams();
      params.set("limit", "50");
      params.set("offset", String(offset));
      if (tradeSide) params.set("side", tradeSide);
      if (tradeQ.trim()) params.set("q", tradeQ.trim());
      if (period === "custom" && customFrom && customTo) {
        params.set("from", new Date(customFrom).toISOString());
        params.set("to", new Date(customTo).toISOString());
      } else if (data?.from && data?.to) {
        params.set("from", String(data.from));
        params.set("to", String(data.to));
      }
      const res = await client.request<{ items: Array<Record<string, unknown>>; total: number }>(
        `/api/v1/source-traders/${traderId}/analytics/trades?${params.toString()}`,
      );
      setTrades((prev) => (offset === 0 ? res.items : [...prev, ...res.items]));
      setTradesTotal(res.total);
      setTradesOffset(offset);
    },
    [client, traderId, tradeSide, tradeQ, period, customFrom, customTo, data],
  );

  useEffect(() => {
    void loadDashboard();
  }, [loadDashboard]);

  useEffect(() => {
    if (data) void loadTrades(0);
  }, [data, tradeSide, loadTrades]);

  async function refreshStats() {
    if (refreshing) return;
    setRefreshing(true);
    try {
      await client.request(`/api/v1/source-traders/${traderId}/analytics/refresh`, { method: "POST" });
      await loadDashboard();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Refresh failed");
    } finally {
      setRefreshing(false);
    }
  }

  const metrics = (data?.metrics ?? {}) as Record<string, unknown>;
  const buyStats = (metrics.buy_stats ?? {}) as Record<string, unknown>;
  const sellStats = (metrics.sell_stats ?? {}) as Record<string, unknown>;
  const pnlPoints = (data?.pnl_points ?? []) as Array<Record<string, unknown>>;
  const chartPoints: EquityPoint[] = pnlPoints.map((p) => ({
    at: String(p.at),
    equity_metric_usd: String(p.cumulative_delta_usd ?? p.economic_pnl_usd ?? "0"),
  }));

  const syncStatus = String(data?.sync_status ?? "pending");
  const syncPhase = String(data?.sync_phase ?? "idle");
  const lastSuccess = data?.last_success_at ? String(data.last_success_at) : null;
  const syncError = data?.last_error ? String(data.last_error) : null;
  const tradesSync = (data?.trades_sync ?? {}) as Record<string, unknown>;
  const backfillIncomplete = tradesSync.backfill_complete === false;
  const hasMoreTrades = tradesSync.has_more_trades === true;

  return (
    <div style={{ display: "grid", gap: 20 }}>
      <header className="panel" style={{ display: "grid", gap: 10 }}>
        <div className="row" style={{ justifyContent: "space-between", flexWrap: "wrap", gap: 12 }}>
          <div>
            <h1 style={{ margin: 0 }}>{alias}</h1>
            <p className="muted" style={{ margin: "4px 0", display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
              <span style={{ fontFamily: "monospace" }}>{wallet}</span>
              <CopyButton value={wallet} label="Copy wallet" />
              <a href={polymarketProfileUrl(wallet)} target="_blank" rel="noreferrer">
                Polymarket profile
              </a>
            </p>
          </div>
          <button type="button" onClick={refreshStats} disabled={refreshing}>
            {refreshing ? "Refreshing…" : "Refresh Statistics"}
          </button>
        </div>
        <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
          <span className="muted">
            Sync: <strong>{syncStatus}</strong>
            {syncPhase ? ` · phase ${syncPhase}` : ""}
            {lastSuccess ? ` · updated ${lastSuccess}` : ""}
          </span>
          {tradesSync.cached_trades_count != null ? (
            <span className="muted">
              Cached fills: <strong>{String(tradesSync.cached_trades_count)}</strong>
              {tradesSync.pages_fetched != null ? ` · ${String(tradesSync.pages_fetched)} API pages` : ""}
            </span>
          ) : null}
          {num(data?.markets_traded) != null ? (
            <span className="muted">
              Markets (unique): <strong>{String(data?.markets_traded)}</strong>
            </span>
          ) : null}
          {data?.all_time_volume_usd != null ? (
            <span className="muted">
              All-time volume: <strong>{usd(String(data.all_time_volume_usd))}</strong>
            </span>
          ) : null}
        </div>
        {hasMoreTrades || (backfillIncomplete && syncPhase === "backfill_partial") ? (
          <p className="warn">
            Trade history backfill in progress
            {tradesSync.stop_reason ? ` (${String(tradesSync.stop_reason)} per worker tick)` : ""}.
            Metrics and recent trades use loaded pages only; worker continues automatically.
          </p>
        ) : null}
        {syncError ? <p className="warn">{syncError}</p> : null}
        {error ? <p className="error">{error}</p> : null}
      </header>

      <section className="panel">
        <PeriodSelector
          period={period}
          onPeriod={setPeriod}
          customFrom={customFrom}
          customTo={customTo}
          onCustomFrom={setCustomFrom}
          onCustomTo={setCustomTo}
          onApply={() => void loadDashboard()}
        />
      </section>

      <EquityChart
        points={chartPoints}
        period={period}
        loading={loading}
        error={error ?? undefined}
        equityLabel={String(data?.pnl_chart_label ?? "Economic PnL delta")}
        equityNote="Cumulative change in Polymarket economic PnL over the selected period (not trade PnL)."
        emptyNote={
          chartPoints.length === 0
            ? "PnL series appears after Refresh Statistics syncs /v2/user-pnl from Polymarket."
            : undefined
        }
      />

      <div className="metrics-grid">
        <MetricCard
          label="Economic PnL (period Δ)"
          value={usd(metrics.economic_pnl_delta as string)}
          hint="From user-pnl series"
        />
        <MetricCard label="Trade PnL (period Δ)" value={usd(metrics.trade_pnl_delta as string)} />
        <MetricCard label="Realized PnL (period Δ)" value={usd(metrics.realized_pnl_delta as string)} />
        <MetricCard label="Unrealized PnL (end)" value={usd(metrics.unrealized_pnl_end as string)} />
        <MetricCard label="Trading volume" value={usd(metrics.trading_volume_usd as string)} hint="Cached trades in period" />
        <MetricCard
          label="Total trades"
          value={String(metrics.total_trades ?? "—")}
          hint="Individual fills in cache (not unique markets)"
        />
        <MetricCard label="Avg trade" value={usd(metrics.average_trade_usd as string)} />
        <MetricCard label="Median trade" value={usd(metrics.median_trade_usd as string)} />
        <MetricCard label="Largest trade" value={usd(metrics.largest_trade_usd as string)} />
        <MetricCard label="Fees (period Δ)" value={usd(metrics.fees_delta as string)} />
        <MetricCard label="Rebates / income (Δ)" value={usd(metrics.wallet_income_delta as string)} />
      </div>

      <TradeSizeBlock title="BUY" stats={buyStats} count={metrics.buy_count} volume={metrics.buy_volume_usd} />
      <TradeSizeBlock title="SELL" stats={sellStats} count={metrics.sell_count} volume={metrics.sell_volume_usd} />

      <BuySellBar buy={num(metrics.buy_count) ?? 0} sell={num(metrics.sell_count) ?? 0} />

      <OpenPositions data={data} />

      <RecentTrades
        trades={trades}
        total={tradesTotal}
        side={tradeSide}
        onSide={setTradeSide}
        q={tradeQ}
        onQ={setTradeQ}
        onSearch={(e: FormEvent) => {
          e.preventDefault();
          void loadTrades(0);
        }}
        onMore={() => void loadTrades(tradesOffset + 50)}
        hasMore={trades.length < tradesTotal}
      />

      <p className="muted" style={{ fontSize: 12 }}>
        Period filter: {querySuffix.replace(/^\?/, "")}
      </p>
    </div>
  );
}

function PeriodSelector({
  period,
  onPeriod,
  customFrom,
  customTo,
  onCustomFrom,
  onCustomTo,
  onApply,
}: {
  period: Period;
  onPeriod: (p: Period) => void;
  customFrom: string;
  customTo: string;
  onCustomFrom: (s: string) => void;
  onCustomTo: (s: string) => void;
  onApply: () => void;
}) {
  return (
    <div style={{ display: "grid", gap: 10 }}>
      <strong>Period</strong>
      <div className="row" style={{ flexWrap: "wrap", gap: 8 }}>
        {PERIODS.map((p) => (
          <button
            key={p.id}
            type="button"
            className={`ghost${period === p.id ? " active" : ""}`}
            onClick={() => onPeriod(p.id)}
          >
            {p.label}
          </button>
        ))}
      </div>
      {period === "custom" ? (
        <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
          <input type="datetime-local" value={customFrom} onChange={(e) => onCustomFrom(e.target.value)} />
          <input type="datetime-local" value={customTo} onChange={(e) => onCustomTo(e.target.value)} />
          <button type="button" onClick={onApply}>
            Apply range
          </button>
        </div>
      ) : null}
    </div>
  );
}

function TradeSizeBlock({
  title,
  stats,
  count,
  volume,
}: {
  title: string;
  stats: Record<string, unknown>;
  count: unknown;
  volume: unknown;
}) {
  const buckets = (stats.buckets ?? []) as Array<[string, number] | { 0: string; 1: number }>;
  return (
    <section className="panel">
      <h2 style={{ marginTop: 0 }}>Trade size — {title}</h2>
      <p className="muted">
        Count {String(count ?? 0)} · Volume {usd(volume as string)}
      </p>
      <div className="metrics-grid" style={{ marginBottom: 12 }}>
        <MetricCard label="Average" value={usd(stats.mean_usd as string)} />
        <MetricCard label="Median" value={usd(stats.median_usd as string)} />
        <MetricCard label="P90" value={usd(stats.p90_usd as string)} />
        <MetricCard label="P99" value={usd(stats.p99_usd as string)} />
        <MetricCard label="Max" value={usd(stats.max_usd as string)} />
      </div>
      <table className="data-table">
        <thead>
          <tr>
            <th>Bucket (USD)</th>
            <th>Count</th>
          </tr>
        </thead>
        <tbody>
          {buckets.map((b) => {
            const label = Array.isArray(b) ? b[0] : b[0];
            const c = Array.isArray(b) ? b[1] : b[1];
            return (
              <tr key={label}>
                <td>{label}</td>
                <td>{c}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </section>
  );
}

function BuySellBar({ buy, sell }: { buy: number; sell: number }) {
  const total = buy + sell || 1;
  const buyPct = (buy / total) * 100;
  return (
    <section className="panel">
      <h2 style={{ marginTop: 0 }}>BUY / SELL</h2>
      <div style={{ display: "flex", height: 24, borderRadius: 4, overflow: "hidden" }}>
        <div style={{ width: `${buyPct}%`, background: "#1f7a4d" }} title={`BUY ${buy}`} />
        <div style={{ flex: 1, background: "#9a4318" }} title={`SELL ${sell}`} />
      </div>
      <p className="muted" style={{ marginBottom: 0 }}>
        BUY {buy} ({buyPct.toFixed(1)}%) · SELL {sell} ({(100 - buyPct).toFixed(1)}%)
      </p>
    </section>
  );
}

function OpenPositions({ data }: { data: Dashboard | null }) {
  const open = (data?.positions_open ?? []) as Array<Record<string, unknown>>;
  const redeemable = (data?.positions_redeemable ?? []) as Array<Record<string, unknown>>;
  const summary = (data?.positions_summary ?? {}) as Record<string, unknown>;
  return (
    <section className="panel">
      <h2 style={{ marginTop: 0 }}>Open positions</h2>
      <p className="muted">
        Open {String(summary.open_count ?? open.length)} · Redeemable {String(summary.redeemable_count ?? redeemable.length)} ·
        Value {usd(summary.total_current_value_usd as string)} · Unrealized {usd(summary.total_unrealized_pnl_usd as string)}
      </p>
      <PositionTable title="Active" rows={open} />
      {redeemable.length > 0 ? <PositionTable title="Redeemable" rows={redeemable} /> : null}
    </section>
  );
}

function PositionTable({ title, rows }: { title: string; rows: Array<Record<string, unknown>> }) {
  if (rows.length === 0) return null;
  const sorted = [...rows].sort(
    (a, b) => Number(b.current_value ?? 0) - Number(a.current_value ?? 0),
  );
  return (
    <>
      <h3>{title}</h3>
      <table className="data-table">
        <thead>
          <tr>
            <th>Market</th>
            <th>Outcome</th>
            <th>Size</th>
            <th>Avg</th>
            <th>Current</th>
            <th>Value</th>
            <th>Unreal. PnL</th>
          </tr>
        </thead>
        <tbody>
          {sorted.slice(0, 25).map((r, i) => (
            <tr key={`${r.token_id ?? i}`}>
              <td>{String(r.title ?? "—")}</td>
              <td>{String(r.outcome ?? "—")}</td>
              <td>{Number(r.current_size ?? r.total_size ?? 0).toFixed(2)}</td>
              <td>{Number(r.avg_price ?? 0).toFixed(3)}</td>
              <td>{Number(r.current_price ?? 0).toFixed(3)}</td>
              <td>{usd(String(r.current_value ?? 0))}</td>
              <td className={pnlClass(Number(r.unrealized_pnl ?? 0))}>{usd(String(r.unrealized_pnl ?? 0))}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}

function RecentTrades({
  trades,
  total,
  side,
  onSide,
  q,
  onQ,
  onSearch,
  onMore,
  hasMore,
}: {
  trades: Array<Record<string, unknown>>;
  total: number;
  side: "" | "buy" | "sell";
  onSide: (s: "" | "buy" | "sell") => void;
  q: string;
  onQ: (s: string) => void;
  onSearch: (e: FormEvent) => void;
  onMore: () => void;
  hasMore: boolean;
}) {
  return (
    <section className="panel">
      <h2 style={{ marginTop: 0 }}>Recent trades</h2>
      <form className="row" style={{ gap: 8, flexWrap: "wrap", marginBottom: 12 }} onSubmit={onSearch}>
        <select value={side} onChange={(e) => onSide(e.target.value as "" | "buy" | "sell")}>
          <option value="">All sides</option>
          <option value="buy">BUY</option>
          <option value="sell">SELL</option>
        </select>
        <input placeholder="Filter market…" value={q} onChange={(e) => onQ(e.target.value)} />
        <button type="submit">Filter</button>
      </form>
      <table className="data-table">
        <thead>
          <tr>
            <th>Time</th>
            <th>Market</th>
            <th>Side</th>
            <th>Price</th>
            <th>Shares</th>
            <th>Notional</th>
          </tr>
        </thead>
        <tbody>
          {trades.map((t, i) => (
            <tr key={`${t.event_ts}-${i}`}>
              <td>{String(t.event_ts)}</td>
              <td>
                {t.market_slug ? (
                  <a href={marketUrl(String(t.market_slug)) ?? "#"} target="_blank" rel="noreferrer">
                    {String(t.market_title ?? "—")}
                  </a>
                ) : (
                  String(t.market_title ?? "—")
                )}
              </td>
              <td>{String(t.side).toUpperCase()}</td>
              <td>{Number(t.price).toFixed(4)}</td>
              <td>{Number(t.size_shares).toFixed(2)}</td>
              <td>{usd(String(t.notional_usd))}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="muted">
        Showing {trades.length} of {total}
      </p>
      {hasMore ? (
        <button type="button" className="ghost" onClick={onMore}>
          Load more
        </button>
      ) : null}
    </section>
  );
}