"use client";

import Link from "next/link";
import { useParams } from "next/navigation";
import { FormEvent, useState } from "react";
import { Desk, useResource } from "@/components/desk";
import { useSession } from "@/lib/session";
import { proportionToPercent } from "@/lib/copy-sizing";

type Utilization = {
  allocated_budget_usd: string;
  deployed_capital_usd: string;
  reserved_capital_usd: string;
  available_budget_usd: string;
  open_positions_count: number;
  enabled: boolean;
  operating_mode: string;
  copied_trades_count: number;
  rejected_trades_count: number;
  last_source_event_at?: string | null;
  last_copied_at?: string | null;
};

export default function StrategyPage() {
  const params = useParams<{ id: string }>();
  return (
    <Desk>
      <Editor id={params.id} />
    </Desk>
  );
}

function Editor({ id }: { id: string }) {
  const { client } = useSession();
  const { data, error, reload } = useResource<Record<string, string | number | boolean | unknown>>(
    `/api/v1/strategies/${id}`,
  );
  const util = useResource<Utilization>(`/api/v1/strategies/${id}/utilization`, { refreshMs: 8000 });
  const [message, setMessage] = useState("");
  const [enableError, setEnableError] = useState("");

  const allocated = Number(util.data?.allocated_budget_usd ?? data?.strategy_budget_usd ?? 0);
  const deployed = Number(util.data?.deployed_capital_usd ?? 0);
  const reserved = Number(util.data?.reserved_capital_usd ?? 0);
  const used = deployed + reserved;
  const pct = allocated > 0 ? Math.min(100, (used / allocated) * 100) : 0;

  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = Object.fromEntries(new FormData(event.currentTarget).entries());
    const body: Record<string, unknown> = {
      ...form,
      copy_buy: form.copy_buy === "on",
      copy_sell: form.copy_sell === "on",
    };
    delete body.enabled;
    if (body.sizing_mode === "proportional" && typeof body.copy_percent === "string" && body.copy_percent) {
      body.copy_percent = body.copy_percent;
      delete body.proportion;
    }
    for (const key of Object.keys(body)) {
      if (body[key] === "") delete body[key];
    }
    if (typeof body.allowed_markets === "string" && body.allowed_markets.trim()) {
      body.allowed_markets = JSON.parse(String(body.allowed_markets));
    } else {
      delete body.allowed_markets;
    }
    await client.request(`/api/v1/strategies/${id}`, { method: "PATCH", body: JSON.stringify(body) });
    setMessage("Saved");
    reload();
    util.reload();
  }

  async function toggleEnable() {
    setEnableError("");
    const next = !data?.enabled;
    if (next) {
      const ok = confirm(
        "Copy trading starts from new source trades after activation. Existing source positions are not copied.\n\nEnable this strategy?",
      );
      if (!ok) return;
    }
    try {
      await client.request(`/api/v1/strategies/${id}/enable`, {
        method: "POST",
        body: JSON.stringify({ enabled: next }),
      });
      reload();
      util.reload();
      setMessage(next ? "Strategy enabled" : "Strategy disabled");
    } catch (e) {
      setEnableError(e instanceof Error ? e.message : "Enable failed");
    }
  }

  async function remove() {
    await client.request(`/api/v1/strategies/${id}`, { method: "DELETE" });
    location.href = "/strategies";
  }

  if (error) return <p className="error">{error}</p>;
  if (!data) return <p>Loading…</p>;

  const proportion = Number(data.proportion ?? 0);
  const copyPercent = data.sizing_mode === "proportional" ? proportionToPercent(proportion) : "";

  return (
    <section style={{ display: "grid", gap: 16, maxWidth: 720 }}>
      <p><Link href="/strategies">← Strategies</Link></p>
      <div className="row">
        <h1 style={{ margin: 0 }}>{String(data.alias)} on {String(data.account_name)}</h1>
        <button type="button" onClick={toggleEnable}>
          {data.enabled ? "Disable strategy" : "Enable strategy (PAPER)"}
        </button>
      </div>
      {enableError && <p className="error">{enableError}</p>}

      {util.data && (
        <BudgetOverview util={util.data} pct={pct} used={used} allocated={allocated} />
      )}

      <form className="panel" onSubmit={save} style={{ display: "grid", gap: 8 }}>
        <h2 style={{ margin: 0 }}>Settings</h2>
        <select name="sizing_mode" defaultValue={String(data.sizing_mode)}>
          <option value="fixed_amount">Fixed amount per trade</option>
          <option value="proportional">Percent of source trade notional</option>
          <option value="multiplier">Multiplier of source notional</option>
        </select>
        <input name="fixed_amount_usd" defaultValue={String(data.fixed_amount_usd ?? "")} placeholder="Fixed USD" />
        <input
          name="copy_percent"
          defaultValue={copyPercent ? String(copyPercent) : String(data.proportion ?? "")}
          placeholder="Copy % of source trade (e.g. 10)"
        />
        <input name="multiplier" defaultValue={String(data.multiplier ?? "")} placeholder="Multiplier" />
        <input name="strategy_budget_usd" defaultValue={String(data.strategy_budget_usd)} placeholder="Strategy budget" />
        <input name="max_position_usd" defaultValue={String(data.max_position_usd)} placeholder="Max per trade" />
        <input name="max_exposure_usd" defaultValue={String(data.max_exposure_usd)} placeholder="Max exposure" />
        <input name="max_open_positions" defaultValue={String(data.max_open_positions)} placeholder="Max open positions" />
        <input name="max_slippage_bps" defaultValue={String(data.max_slippage_bps)} placeholder="Max slippage bps" />
        <input name="min_order_usd" defaultValue={String(data.min_order_usd)} placeholder="Min order USD" />
        <input name="cooldown_seconds" defaultValue={String(data.cooldown_seconds)} placeholder="Cooldown seconds" />
        <textarea name="allowed_markets" defaultValue={JSON.stringify(data.allowed_markets ?? [])} />
        <label><input type="checkbox" name="copy_buy" defaultChecked={Boolean(data.copy_buy)} /> Copy buys</label>
        <label><input type="checkbox" name="copy_sell" defaultChecked={Boolean(data.copy_sell)} /> Copy sells</label>
        <p className="muted" style={{ margin: 0 }}>
          New strategies start disabled. Save edits, then use Enable above. SELL still runs when budget is full.
        </p>
        <div className="filters">
          <button>Save strategy</button>
          <button type="button" className="ghost" onClick={remove}>Delete</button>
        </div>
        {message && <div className="ok">{message}</div>}
      </form>
    </section>
  );
}

function BudgetOverview({
  util,
  pct,
  used,
  allocated,
}: {
  util: Utilization;
  pct: number;
  used: number;
  allocated: number;
}) {
  const warn = pct >= 80 && pct < 100;
  const full = pct >= 100;
  return (
    <div className="panel" style={{ display: "grid", gap: 8 }}>
      <h2 style={{ margin: 0 }}>Budget overview</h2>
      <div>
        Status: <strong>{util.enabled ? "Enabled" : "Disabled"}</strong>
        {" · "}
        Mode: <strong>{util.operating_mode}</strong>
      </div>
      <div className="muted">
        Allocated: ${util.allocated_budget_usd}
        {" · "}
        Deployed: ${util.deployed_capital_usd}
        {" · "}
        Reserved: ${util.reserved_capital_usd}
        {" · "}
        Available: ${util.available_budget_usd}
      </div>
      <div style={{ background: "#1e293b", borderRadius: 4, height: 10, overflow: "hidden" }}>
        <div
          style={{
            width: `${pct}%`,
            height: "100%",
            background: full ? "#dc2626" : warn ? "#f59e0b" : "#22c55e",
          }}
        />
      </div>
      {(warn || full) && (
        <p className={full ? "error" : "warn"} style={{ margin: 0 }}>
          {full
            ? "Budget cap reached — new BUY copies will be rejected; SELL still allowed."
            : "Over 80% of strategy budget is in use."}
        </p>
      )}
      <div>
        Open positions: {util.open_positions_count}
        {" · "}
        Copied: {util.copied_trades_count}
        {" · "}
        Rejected: {util.rejected_trades_count}
      </div>
      <div className="muted">
        Last source event: {util.last_source_event_at ?? "—"}
        {" · "}
        Last copy: {util.last_copied_at ?? "—"}
      </div>
    </div>
  );
}
