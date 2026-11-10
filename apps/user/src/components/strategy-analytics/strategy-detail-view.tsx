"use client";

import Link from "next/link";
import { FormEvent, useState } from "react";
import { useResource } from "@/components/desk";
import { SimpleFieldLabel } from "@/components/simple-field-label";
import { useToast } from "@/components/toast";
import { explainDecisionReason } from "@/lib/decision-reason";
import { proportionToPercent } from "@/lib/copy-sizing";
import { pnlClass, pct, usd } from "@/lib/strategy-analytics-format";
import { useSession } from "@/lib/session";
import { EquityChart, type EquityPoint } from "./equity-chart";

type Tab = "overview" | "positions" | "activity" | "performance" | "settings";

type Metrics = {
  allocated_capital_usd: string;
  invested_capital_usd: string;
  available_capital_usd: string;
  realized_pnl_usd: string;
  unrealized_pnl_usd: string;
  total_pnl_usd: string;
  roi_percent?: string | null;
  open_positions_count: number;
  total_trades_count: number;
  equity: { strategy_equity_usd: string; equity_label: string; note: string };
};

type Overview = {
  metrics: Metrics;
  recent_activity?: ActivityRow[];
  top_positions?: PositionRow[];
  bottom_positions?: PositionRow[];
};

type PositionRow = {
  id: string;
  market_title?: string | null;
  outcome?: string | null;
  status: string;
  open_shares: string;
  cost_basis_usd: string;
  market_value_usd: string;
  unrealized_pnl_usd: string;
  realized_pnl_usd: string;
  return_percent?: string | null;
  mark_source?: string;
  mark_observed_at?: string | null;
  closed_at?: string | null;
  avg_exit_price?: string | null;
  proceeds_usd?: string | null;
  position_fees_usd?: string | null;
};

type ActivityRow = {
  decision_id: string;
  decision_status: string;
  execution_status: string;
  side: string;
  market_title?: string | null;
  decided_at: string;
  filled_at?: string | null;
  source_notional_usd?: string | null;
  fill_price?: string | null;
  reason?: string | null;
  reason_title?: string | null;
  reason_detail?: string | null;
};

export function StrategyDetailView({ id }: { id: string }) {
  const [tab, setTab] = useState<Tab>("overview");
  const [period, setPeriod] = useState("1M");
  const [posFilter, setPosFilter] = useState<"open" | "closed" | "all">("open");
  const [expanded, setExpanded] = useState<string | null>(null);
  const { client } = useSession();
  const toast = useToast();

  const meta = useResource<Record<string, unknown>>(`/api/v1/strategies/${id}`);
  const overview = useResource<Overview>(`/api/v1/strategies/${id}/analytics/overview`, {
    enabled: tab === "overview",
    refreshMs: 12_000,
  });
  const equityPath = `/api/v1/strategies/${id}/analytics/equity?period=${encodeURIComponent(period)}`;
  const equity = useResource<{ points: EquityPoint[]; empty_note?: string; equity_label?: string; equity_note?: string }>(
    equityPath,
    { enabled: tab === "overview", refreshMs: 30_000 },
  );
  const positions = useResource<{ items: PositionRow[]; total: number }>(
    `/api/v1/strategies/${id}/analytics/positions?status=${posFilter}&limit=50`,
    { enabled: tab === "positions" },
  );
  const activity = useResource<{ items: ActivityRow[] }>(
    `/api/v1/strategies/${id}/analytics/activity?limit=50`,
    { enabled: tab === "activity" },
  );
  const performance = useResource<Record<string, string | number | null>>(
    `/api/v1/strategies/${id}/analytics/performance`,
    { enabled: tab === "performance" },
  );

  async function toggleEnable() {
    if (!meta.data) return;
    const next = !meta.data.enabled;
    if (next && !confirm("Enable copy trading for this strategy?")) return;
    try {
      await client.request(`/api/v1/strategies/${id}/enable`, {
        method: "POST",
        body: JSON.stringify({ enabled: next }),
      });
      meta.reload();
      toast.success(next ? "Strategy enabled" : "Strategy disabled");
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "Enable failed");
    }
  }

  if (meta.error) return <p className="error">{meta.error}</p>;
  if (!meta.data) return <p>Loading…</p>;

  const m = overview.data?.metrics;

  return (
    <section className="strategy-detail" style={{ display: "grid", gap: 16, maxWidth: 960 }}>
      <p style={{ margin: 0 }}>
        <Link href="/strategies">← Strategies</Link>
      </p>
      <div className="row">
        <h1 style={{ margin: 0 }}>
          {String(meta.data.display_name ?? meta.data.alias)} · {String(meta.data.account_name ?? "")}
        </h1>
        <button type="button" onClick={() => void toggleEnable()}>
          {meta.data.enabled ? "Disable strategy" : "Enable strategy (PAPER)"}
        </button>
      </div>

      <nav className="filters" aria-label="Strategy sections">
        {(
          [
            ["overview", "Overview"],
            ["positions", "Positions"],
            ["activity", "Activity"],
            ["performance", "Performance"],
            ["settings", "Settings"],
          ] as const
        ).map(([key, label]) => (
          <button
            key={key}
            type="button"
            className={tab === key ? "" : "ghost"}
            onClick={() => setTab(key)}
          >
            {label}
          </button>
        ))}
      </nav>

      {tab === "overview" && (
        <div style={{ display: "grid", gap: 16 }}>
          {overview.error && <p className="error">{overview.error}</p>}
          {!overview.data && !overview.error && <p className="muted">Loading metrics…</p>}
          {overview.data && m && (
            <>
              <div className="metric-grid">
                <MetricCard label="Strategy equity" value={usd(m.equity.strategy_equity_usd)} hint={m.equity.equity_label} />
                <MetricCard label="Total P&L" value={usd(m.total_pnl_usd)} className={pnlClass(Number(m.total_pnl_usd))} />
                <MetricCard label="ROI" value={m.roi_percent != null ? pct(String(m.roi_percent)) : "—"} />
                <MetricCard label="Allocated" value={usd(m.allocated_capital_usd)} />
                <MetricCard label="Invested" value={usd(m.invested_capital_usd)} />
                <MetricCard label="Available" value={usd(m.available_capital_usd)} />
                <MetricCard label="Realized P&L" value={usd(m.realized_pnl_usd)} className={pnlClass(Number(m.realized_pnl_usd))} />
                <MetricCard label="Unrealized P&L" value={usd(m.unrealized_pnl_usd)} className={pnlClass(Number(m.unrealized_pnl_usd))} />
                <MetricCard label="Open positions" value={String(m.open_positions_count)} />
                <MetricCard label="Decisions" value={String(m.total_trades_count)} />
              </div>
              <p className="muted" style={{ margin: 0, fontSize: 13 }}>
                {m.equity.note}
              </p>
              <div className="filters">
                {["1D", "1W", "1M", "3M", "6M", "1Y", "ALL"].map((p) => (
                  <button key={p} type="button" className={period === p ? "" : "ghost"} onClick={() => setPeriod(p)}>
                    {p}
                  </button>
                ))}
              </div>
              <EquityChart
                points={equity.data?.points ?? []}
                loading={!equity.data && !equity.error}
                error={equity.error}
                emptyNote={equity.data?.empty_note}
                equityLabel={equity.data?.equity_label}
                equityNote={equity.data?.equity_note}
              />
              <MiniPositions title="Top performers" rows={overview.data.top_positions ?? []} />
              <MiniPositions title="Weakest open positions" rows={overview.data.bottom_positions ?? []} />
              <RecentActivity rows={overview.data.recent_activity ?? []} />
            </>
          )}
        </div>
      )}

      {tab === "positions" && (
        <>
          <div className="filters">
            {(["open", "closed", "all"] as const).map((f) => (
              <button key={f} type="button" className={posFilter === f ? "" : "ghost"} onClick={() => setPosFilter(f)}>
                {f === "open" ? "Open" : f === "closed" ? "Closed" : "All"}
              </button>
            ))}
          </div>
          <PositionsTable loading={!positions.data && !positions.error} error={positions.error} rows={positions.data?.items ?? []} />
        </>
      )}

      {tab === "activity" && (
        <ActivityTable
          rows={activity.data?.items ?? []}
          loading={!activity.data && !activity.error}
          error={activity.error}
          expanded={expanded}
          onToggle={(id) => setExpanded((cur) => (cur === id ? null : id))}
        />
      )}

      {tab === "performance" && (
        <PerformancePanel data={performance.data ?? undefined} error={performance.error} loading={!performance.data && !performance.error} />
      )}

      {tab === "settings" && (
        <StrategySettingsForm id={id} data={meta.data} onSaved={() => meta.reload()} />
      )}
    </section>
  );
}

function MetricCard({ label, value, hint, className }: { label: string; value: string; hint?: string; className?: string }) {
  return (
    <div className="panel metric-card">
      <span className="muted" style={{ fontSize: 12 }}>
        {label}
      </span>
      <strong className={className}>{value}</strong>
      {hint && (
        <span className="muted" style={{ fontSize: 11 }}>
          {hint}
        </span>
      )}
    </div>
  );
}

function MiniPositions({ title, rows }: { title: string; rows: PositionRow[] }) {
  if (!rows.length) return null;
  return (
    <div className="panel">
      <h3 style={{ marginTop: 0 }}>{title}</h3>
      <table className="data-table">
        <thead>
          <tr>
            <th>Market</th>
            <th>Unrealized</th>
            <th>Value</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.id}>
              <td>{r.market_title ?? r.id.slice(0, 8)}</td>
              <td className={pnlClass(Number(r.unrealized_pnl_usd))}>{usd(r.unrealized_pnl_usd)}</td>
              <td>{usd(r.market_value_usd)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function RecentActivity({ rows }: { rows: ActivityRow[] }) {
  if (!rows.length) return <p className="muted">No copy activity yet.</p>;
  return (
    <div className="panel">
      <h3 style={{ marginTop: 0 }}>Recent activity</h3>
      <ul className="timeline" style={{ margin: 0, paddingLeft: 18 }}>
        {rows.slice(0, 5).map((r) => (
          <li key={r.decision_id}>
            <span className="pill">{r.execution_status}</span> {r.side.toUpperCase()}{" "}
            {r.market_title ?? "—"} · {new Date(r.decided_at).toLocaleString()}
          </li>
        ))}
      </ul>
    </div>
  );
}

function PositionsTable({ rows, loading, error }: { rows: PositionRow[]; loading?: boolean; error?: string }) {
  if (loading) return <p className="muted">Loading positions…</p>;
  if (error) return <p className="error">{error}</p>;
  if (!rows.length) return <p className="muted">No open positions for this strategy.</p>;
  return (
    <table className="data-table panel" style={{ width: "100%" }}>
      <thead>
        <tr>
          <th>Market</th>
          <th>Status</th>
          <th>Shares</th>
          <th>Cost</th>
          <th>Value / proceeds</th>
          <th>P&L</th>
          <th>Return</th>
          <th>Mark</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.id}>
            <td>{r.market_title ?? "—"}</td>
            <td>{r.status}</td>
            <td>{r.open_shares}</td>
            <td>{usd(r.cost_basis_usd)}</td>
            <td>{usd(r.status === "closed" ? (r.proceeds_usd ?? r.market_value_usd) : r.market_value_usd)}</td>
            <td className={pnlClass(Number(r.status === "closed" ? r.realized_pnl_usd : r.unrealized_pnl_usd))}>
              {usd(r.status === "closed" ? r.realized_pnl_usd : r.unrealized_pnl_usd)}
            </td>
            <td>{r.return_percent != null ? pct(r.return_percent) : "—"}</td>
            <td className="muted" style={{ fontSize: 11 }}>
              {r.mark_source ?? "—"}
              {r.mark_observed_at ? ` · ${new Date(r.mark_observed_at).toLocaleDateString()}` : ""}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function ActivityTable({
  rows,
  loading,
  error,
  expanded,
  onToggle,
}: {
  rows: ActivityRow[];
  loading?: boolean;
  error?: string;
  expanded: string | null;
  onToggle: (id: string) => void;
}) {
  if (loading) return <p className="muted">Loading activity…</p>;
  if (error) return <p className="error">{error}</p>;
  if (!rows.length) return <p className="muted">No copy decisions yet.</p>;
  return (
    <div className="panel" style={{ display: "grid", gap: 8 }}>
      {rows.map((r) => (
        <div key={r.decision_id} className="activity-card" style={{ borderBottom: "1px solid #1e293b", paddingBottom: 8 }}>
          <div className="row">
            <strong>
              {r.side.toUpperCase()} · {r.execution_status}
            </strong>
            <button type="button" className="ghost" onClick={() => onToggle(r.decision_id)}>
              {expanded === r.decision_id ? "Hide" : "Details"}
            </button>
          </div>
          <p className="muted" style={{ margin: "4px 0" }}>
            {r.market_title ?? "—"} · decided {new Date(r.decided_at).toLocaleString()}
          </p>
          {expanded === r.decision_id && (
            <dl style={{ margin: 0, fontSize: 13 }}>
              <dt>Decision</dt>
              <dd>
                {r.decision_status}
                {r.reason ? ` — ${r.reason}` : ""}
              </dd>
              {(r.reason_title ?? r.reason_detail ?? explainDecisionReason(r.reason).title) && (
                <>
                  <dt>Why skipped</dt>
                  <dd>
                    {r.reason_title ?? explainDecisionReason(r.reason).title}
                    {(r.reason_detail ?? explainDecisionReason(r.reason).detail) && (
                      <span className="muted" style={{ display: "block", marginTop: 4 }}>
                        {r.reason_detail ?? explainDecisionReason(r.reason).detail}
                      </span>
                    )}
                  </dd>
                </>
              )}
              <dt>Source notional</dt>
              <dd>{r.source_notional_usd != null ? usd(r.source_notional_usd) : "—"}</dd>
              <dt>Fill</dt>
              <dd>
                {r.filled_at ? new Date(r.filled_at).toLocaleString() : "—"}
                {r.fill_price != null ? ` @ ${usd(r.fill_price, 4)}` : ""}
              </dd>
            </dl>
          )}
        </div>
      ))}
    </div>
  );
}

function PerformancePanel({
  data,
  loading,
  error,
}: {
  data?: Record<string, string | number | null>;
  loading?: boolean;
  error?: string;
}) {
  if (loading) return <p className="muted">Loading performance…</p>;
  if (error) return <p className="error">{error}</p>;
  if (!data) return null;
  return (
    <div className="panel metric-grid">
      <MetricCard label="Total return" value={usd(String(data.total_return_usd ?? 0))} />
      <MetricCard label="ROI" value={data.roi_percent != null ? pct(String(data.roi_percent)) : "—"} />
      <MetricCard label="Win rate" value={data.win_rate_percent != null ? pct(String(data.win_rate_percent)) : "—"} />
      <MetricCard label="Profit factor" value={data.profit_factor != null ? String(data.profit_factor) : "—"} />
      <MetricCard label="Max drawdown" value={data.max_drawdown_percent != null ? pct(String(data.max_drawdown_percent)) : "—"} />
      <MetricCard label="Fees" value={usd(String(data.fees_usd ?? 0))} />
      {data.sample_size_note && (
        <p className="warn" style={{ gridColumn: "1 / -1", margin: 0 }}>
          {String(data.sample_size_note)}
        </p>
      )}
    </div>
  );
}

function StrategySettingsForm({
  id,
  data,
  onSaved,
}: {
  id: string;
  data: Record<string, unknown>;
  onSaved: () => void;
}) {
  const { client } = useSession();
  const toast = useToast();
  const [formKey, setFormKey] = useState(0);
  const proportion = Number(data.proportion ?? 0);
  const copyPercent = data.sizing_mode === "proportional" ? proportionToPercent(proportion) : "";
  const allowedDefault =
    Array.isArray(data.allowed_markets) ? JSON.stringify(data.allowed_markets, null, 2) : "[]";

  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = Object.fromEntries(new FormData(event.currentTarget).entries());
    const body: Record<string, unknown> = {
      ...form,
      copy_buy: form.copy_buy === "on",
      copy_sell: form.copy_sell === "on",
    };
    delete body.enabled;
    for (const key of Object.keys(body)) {
      if (body[key] === "") delete body[key];
    }
    if (typeof body.allowed_markets === "string" && body.allowed_markets.trim()) {
      body.allowed_markets = JSON.parse(String(body.allowed_markets));
    } else {
      delete body.allowed_markets;
    }
    try {
      if (!confirm("Saved settings apply only to new copy decisions. Continue?")) {
        return;
      }
      await client.request(`/api/v1/strategies/${id}`, { method: "PATCH", body: JSON.stringify(body) });
      toast.success("Strategy settings saved");
      onSaved();
      setFormKey((k) => k + 1);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "Save failed");
    }
  }

  async function remove() {
    if (!confirm("Delete this strategy?")) return;
    try {
      await client.request(`/api/v1/strategies/${id}`, { method: "DELETE" });
      toast.info("Strategy deleted");
      location.href = "/strategies";
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "Delete failed");
    }
  }

  return (
    <form key={formKey} className="panel form-stack" onSubmit={save}>
      <h2 style={{ margin: 0 }}>Settings</h2>
      <p className="muted" style={{ margin: 0, fontSize: 13 }}>
        Enable / disable is controlled from the header. Changes here apply to new decisions only.
      </p>
      <fieldset style={{ border: 0, padding: 0, margin: 0, display: "grid", gap: 12 }}>
        <legend className="muted" style={{ fontSize: 12 }}>
          Identity
        </legend>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="display_name">Strategy name</SimpleFieldLabel>
          <input id="display_name" name="display_name" defaultValue={String(data.display_name ?? data.alias ?? "")} />
        </div>
      </fieldset>
      <fieldset style={{ border: 0, padding: 0, margin: 0, display: "grid", gap: 12 }}>
        <legend className="muted" style={{ fontSize: 12 }}>
          Capital allocation (ledger)
        </legend>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="allocation_mode">Allocation mode</SimpleFieldLabel>
          <select id="allocation_mode" name="allocation_mode" defaultValue={String(data.allocation_mode ?? "fixed_usd")}>
            <option value="fixed_usd">Fixed USD cap</option>
            <option value="percent_of_account">Percent of account equity</option>
          </select>
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="strategy_budget_usd">Budget (USD) or percent cap denominator</SimpleFieldLabel>
          <input id="strategy_budget_usd" name="strategy_budget_usd" defaultValue={String(data.strategy_budget_usd)} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="allocation_percent" optional>
            Allocation percent (when using % of account)
          </SimpleFieldLabel>
          <input
            id="allocation_percent"
            name="allocation_percent"
            defaultValue={data.allocation_percent != null ? String(data.allocation_percent) : ""}
          />
        </div>
      </fieldset>
      <fieldset style={{ border: 0, padding: 0, margin: 0, display: "grid", gap: 12 }}>
        <legend className="muted" style={{ fontSize: 12 }}>
          Copy sizing (per trade)
        </legend>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="sizing_mode">Sizing mode</SimpleFieldLabel>
          <select id="sizing_mode" name="sizing_mode" defaultValue={String(data.sizing_mode)}>
            <option value="portfolio_ratio">Portfolio-relative (leader % × your allocation)</option>
            <option value="fixed_amount">Fixed USD per trade</option>
            <option value="proportional">Percent of source trade notional</option>
            <option value="multiplier">Multiplier of source notional</option>
          </select>
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="fixed_amount_usd" optional>
            Fixed amount (USD)
          </SimpleFieldLabel>
          <input id="fixed_amount_usd" name="fixed_amount_usd" defaultValue={String(data.fixed_amount_usd ?? "")} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="copy_percent" optional>
            Copy % of source trade notional
          </SimpleFieldLabel>
          <input id="copy_percent" name="copy_percent" defaultValue={copyPercent ? String(copyPercent) : ""} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="multiplier" optional>
            Multiplier
          </SimpleFieldLabel>
          <input id="multiplier" name="multiplier" defaultValue={String(data.multiplier ?? "")} />
        </div>
      </fieldset>
      <fieldset style={{ border: 0, padding: 0, margin: 0, display: "grid", gap: 12 }}>
        <legend className="muted" style={{ fontSize: 12 }}>
          Risk limits
        </legend>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="max_position_usd">Max position / trade (USD)</SimpleFieldLabel>
          <input id="max_position_usd" name="max_position_usd" defaultValue={String(data.max_position_usd)} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="max_exposure_usd">Max exposure (USD)</SimpleFieldLabel>
          <input id="max_exposure_usd" name="max_exposure_usd" defaultValue={String(data.max_exposure_usd)} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="max_open_positions">Max open positions</SimpleFieldLabel>
          <input id="max_open_positions" name="max_open_positions" defaultValue={String(data.max_open_positions)} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="min_order_usd">Minimum order (USD)</SimpleFieldLabel>
          <input id="min_order_usd" name="min_order_usd" defaultValue={String(data.min_order_usd)} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="max_slippage_bps">Max slippage (bps)</SimpleFieldLabel>
          <input id="max_slippage_bps" name="max_slippage_bps" defaultValue={String(data.max_slippage_bps)} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="cooldown_seconds">Cooldown between decisions (seconds)</SimpleFieldLabel>
          <input id="cooldown_seconds" name="cooldown_seconds" defaultValue={String(data.cooldown_seconds ?? 0)} />
        </div>
      </fieldset>
      <fieldset style={{ border: 0, padding: 0, margin: 0, display: "grid", gap: 12 }}>
        <legend className="muted" style={{ fontSize: 12 }}>
          Markets &amp; sides
        </legend>
        <label>
          <input type="checkbox" name="copy_buy" defaultChecked={Boolean(data.copy_buy)} /> Copy buys
        </label>
        <label>
          <input type="checkbox" name="copy_sell" defaultChecked={Boolean(data.copy_sell)} /> Copy sells
        </label>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="allowed_markets" optional>
            Allowed markets (JSON array, empty = all)
          </SimpleFieldLabel>
          <textarea id="allowed_markets" name="allowed_markets" rows={3} defaultValue={allowedDefault} />
        </div>
      </fieldset>
      <div className="filters">
        <button type="submit">Save changes</button>
        <button type="button" className="ghost" onClick={() => setFormKey((k) => k + 1)}>
          Cancel
        </button>
        <button type="button" className="ghost" onClick={() => void remove()}>
          Delete
        </button>
      </div>
    </form>
  );
}
