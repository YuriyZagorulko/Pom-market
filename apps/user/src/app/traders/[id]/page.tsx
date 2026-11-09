"use client";

import { useParams } from "next/navigation";
import { FormEvent } from "react";
import { Desk, useResource } from "@/components/desk";
import { useSession } from "@/lib/session";

export default function TraderPage() {
  const params = useParams<{ id: string }>();
  return <Desk><Detail id={params.id} /></Desk>;
}

function Detail({ id }: { id: string }) {
  const { client } = useSession();
  const trader = useResource<Record<string, unknown>>(`/api/v1/source-traders/${id}`);
  const accounts = useResource<Array<Record<string, string>>>("/api/v1/accounts");
  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    await client.request(`/api/v1/source-traders/${id}`, {
      method: "PATCH",
      body: JSON.stringify({ alias: data.get("alias"), monitoring_enabled: data.get("monitoring_enabled") === "on" }),
    });
    trader.reload();
  }
  async function bind(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const body = Object.fromEntries(new FormData(event.currentTarget).entries());
    body.source_trader_id = id;
    const created = await client.request<{ id: string }>("/api/v1/strategies", { method: "POST", body: JSON.stringify(body) });
    location.href = `/strategies/${created.id}`;
  }
  async function remove() {
    await client.request(`/api/v1/source-traders/${id}`, { method: "DELETE" });
    location.href = "/traders";
  }
  if (!trader.data) return <p>{trader.error || "Loading…"}</p>;
  return (
    <section style={{ display: "grid", gap: 16 }}>
      <h1>{String(trader.data.alias)}</h1>
      <div className="panel">
        <div>{String(trader.data.wallet_address)}</div>
        <div>Detected {String(trader.data.observed_trade_count)} · Copied {String(trader.data.copied_trade_count)} · Rejected {String(trader.data.rejected_trade_count)}</div>
        <div>Last trade {String(trader.data.last_trade_at ?? "none")}</div>
        <pre>{JSON.stringify(trader.data.profile, null, 2)}</pre>
        <pre>Linked accounts: {JSON.stringify(trader.data.links, null, 2)}</pre>
        <button className="ghost" onClick={remove}>Remove wallet</button>
      </div>
      <form className="panel" onSubmit={save}>
        <input name="alias" defaultValue={String(trader.data.alias)} />
        <label><input type="checkbox" name="monitoring_enabled" defaultChecked={Boolean(trader.data.monitoring_enabled)} /> Monitoring enabled</label>
        <button>Save</button>
      </form>
      <form className="panel" onSubmit={bind} style={{ display: "grid", gap: 8 }}>
        <h2>Copy onto an account</h2>
        <select name="trading_account_id" required>
          {accounts.data?.map((account) => <option key={account.id} value={account.id}>{account.name}</option>)}
        </select>
        <select name="sizing_mode" defaultValue="fixed_amount">
          <option value="fixed_amount">Fixed amount</option>
          <option value="proportional">Proportional</option>
          <option value="multiplier">Multiplier</option>
        </select>
        <input name="fixed_amount_usd" defaultValue="25" />
        <input name="strategy_budget_usd" defaultValue="100" />
        <button>Create disabled strategy</button>
      </form>
    </section>
  );
}
