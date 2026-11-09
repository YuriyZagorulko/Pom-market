"use client";

import { useParams } from "next/navigation";
import { FormEvent, useState } from "react";
import { Desk, useResource } from "@/components/desk";
import { useSession } from "@/lib/session";

export default function StrategyPage() {
  const params = useParams<{ id: string }>();
  return <Desk><Editor id={params.id} /></Desk>;
}

function Editor({ id }: { id: string }) {
  const { client } = useSession();
  const { data, error, reload } = useResource<Record<string, string | number | boolean | unknown>>(`/api/v1/strategies/${id}`);
  const [message, setMessage] = useState("");
  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = Object.fromEntries(new FormData(event.currentTarget).entries());
    const body: Record<string, unknown> = {
      ...form,
      copy_buy: form.copy_buy === "on",
      copy_sell: form.copy_sell === "on",
      enabled: form.enabled === "on",
    };
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
  }
  async function remove() {
    await client.request(`/api/v1/strategies/${id}`, { method: "DELETE" });
    location.href = "/strategies";
  }
  if (error) return <p className="error">{error}</p>;
  if (!data) return <p>Loading…</p>;
  return (
    <form className="panel" onSubmit={save} style={{ display: "grid", gap: 8, maxWidth: 640 }}>
      <h1>{String(data.alias)} on {String(data.account_name)}</h1>
      <select name="sizing_mode" defaultValue={String(data.sizing_mode)}>
        <option value="fixed_amount">Fixed amount</option>
        <option value="proportional">Proportional</option>
        <option value="multiplier">Multiplier</option>
      </select>
      <input name="fixed_amount_usd" defaultValue={String(data.fixed_amount_usd ?? "")} placeholder="Fixed USD" />
      <input name="proportion" defaultValue={String(data.proportion ?? "")} placeholder="Proportion, e.g. 0.25" />
      <input name="multiplier" defaultValue={String(data.multiplier ?? "")} placeholder="Multiplier" />
      <input name="strategy_budget_usd" defaultValue={String(data.strategy_budget_usd)} placeholder="Strategy budget" />
      <input name="max_position_usd" defaultValue={String(data.max_position_usd)} placeholder="Max position" />
      <input name="max_exposure_usd" defaultValue={String(data.max_exposure_usd)} placeholder="Max exposure" />
      <input name="max_open_positions" defaultValue={String(data.max_open_positions)} placeholder="Max open positions" />
      <input name="max_slippage_bps" defaultValue={String(data.max_slippage_bps)} placeholder="Max slippage bps" />
      <input name="min_order_usd" defaultValue={String(data.min_order_usd)} placeholder="Min order USD" />
      <input name="cooldown_seconds" defaultValue={String(data.cooldown_seconds)} placeholder="Cooldown seconds" />
      <textarea name="allowed_markets" defaultValue={JSON.stringify(data.allowed_markets ?? [])} />
      <label><input type="checkbox" name="copy_buy" defaultChecked={Boolean(data.copy_buy)} /> Copy buys</label>
      <label><input type="checkbox" name="copy_sell" defaultChecked={Boolean(data.copy_sell)} /> Copy sells</label>
      <label><input type="checkbox" name="enabled" defaultChecked={Boolean(data.enabled)} /> Enabled</label>
      <div className="filters"><button>Save strategy</button><button type="button" className="ghost" onClick={remove}>Delete</button></div>
      {message && <div className="ok">{message}</div>}
    </form>
  );
}
