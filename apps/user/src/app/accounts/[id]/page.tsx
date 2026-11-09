"use client";

import { useParams } from "next/navigation";
import { FormEvent, useState } from "react";
import { Desk, useResource } from "@/components/desk";
import { useSession } from "@/lib/session";

export default function AccountPage() {
  const params = useParams<{ id: string }>();
  return <Desk><Detail id={params.id} /></Desk>;
}

function Detail({ id }: { id: string }) {
  const { client } = useSession();
  const { data, error, reload } = useResource<{ account: Record<string, string | null>; risk: Record<string, string | number>; credentials_stored: boolean }>(`/api/v1/accounts/${id}`);
  const [message, setMessage] = useState("");
  const [report, setReport] = useState<Record<string, unknown> | null>(null);
  async function test() {
    setReport(await client.request(`/api/v1/accounts/${id}/test`, { method: "POST" }));
    reload();
  }
  async function mode(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = Object.fromEntries(new FormData(event.currentTarget).entries());
    const body: Record<string, unknown> = { ...form, confirm_live: form.confirm_live === "on" };
    await client.request(`/api/v1/accounts/${id}/mode`, { method: "POST", body: JSON.stringify(body) });
    setMessage("Mode updated");
    reload();
  }
  async function risk(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = Object.fromEntries(new FormData(event.currentTarget).entries());
    const body: Record<string, unknown> = { ...form, emergency_stop: form.emergency_stop === "on" };
    await client.request(`/api/v1/accounts/${id}/risk`, { method: "PATCH", body: JSON.stringify(body) });
    setMessage("Risk limits saved");
    reload();
  }
  async function rename(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const body = Object.fromEntries(new FormData(event.currentTarget).entries());
    await client.request(`/api/v1/accounts/${id}`, { method: "PATCH", body: JSON.stringify(body) });
    reload();
  }
  async function disconnect() {
    await client.request(`/api/v1/accounts/${id}/disconnect`, { method: "POST" });
    reload();
  }
  async function remove() {
    if (!confirm("Delete this account and its strategies? History attached to it is removed with it.")) return;
    await client.request(`/api/v1/accounts/${id}`, { method: "DELETE" });
    location.href = "/accounts";
  }
  if (error) return <p className="error">{error}</p>;
  if (!data) return <p>Loading…</p>;
  const account = data.account;
  return (
    <section style={{ display: "grid", gap: 16 }}>
      <h1>{account.name}</h1>
      <div className="panel">
        <div>Signer {account.signer_address}</div>
        <div>Funder {account.funder_address}</div>
        <div>Signature {account.signature_type} · Mode {account.operating_mode} · {account.status}</div>
        <div>Exchange balance {account.last_exchange_balance_usd ?? "not checked"} {account.last_connection_error && <span className="error"> · {account.last_connection_error}</span>}</div>
        <div>Credentials stored: {data.credentials_stored ? "yes" : "no"}. The key itself is never shown.</div>
        <div className="filters">
          <button onClick={test}>Test connection</button>
          <button className="ghost" onClick={disconnect}>Disconnect</button>
          <button className="ghost" onClick={remove}>Delete</button>
        </div>
        {report && <pre>{JSON.stringify(report, null, 2)}</pre>}
      </div>
      <form className="panel" onSubmit={rename}><input name="name" defaultValue={String(account.name)} /><button>Rename</button></form>
      <form className="panel" onSubmit={mode} style={{ display: "grid", gap: 8 }}>
        <h2>Operating mode</h2>
        <select name="mode" defaultValue={String(account.operating_mode)}>
          <option value="monitoring">Monitoring</option>
          <option value="paper_trading">Paper trading</option>
          <option value="live_trading">Live trading</option>
        </select>
        <label><input name="confirm_live" type="checkbox" /> I understand live orders spend real funds</label>
        <input name="confirmation" placeholder='Type ENABLE LIVE TRADING for live mode' />
        <button>Update mode</button>
      </form>
      <form className="panel" onSubmit={risk} style={{ display: "grid", gap: 8 }}>
        <h2>Account risk</h2>
        <input name="max_order_usd" defaultValue={String(data.risk.max_order_usd)} placeholder="Max order USD" />
        <input name="max_daily_loss_usd" defaultValue={String(data.risk.max_daily_loss_usd)} placeholder="Max daily loss" />
        <input name="max_exposure_usd" defaultValue={String(data.risk.max_exposure_usd)} placeholder="Max exposure" />
        <input name="max_slippage_bps" defaultValue={String(data.risk.max_slippage_bps)} placeholder="Max slippage bps" />
        <input name="paper_cash_usd" defaultValue={String(data.risk.paper_cash_usd)} placeholder="Paper cash" />
        <label><input name="emergency_stop" type="checkbox" defaultChecked={Boolean(account.emergency_stop)} /> Emergency stop</label>
        <button>Save risk</button>
      </form>
      {message && <div className="ok">{message}</div>}
    </section>
  );
}
