"use client";

import Link from "next/link";
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
  const trader = useResource<Record<string, unknown>>(`/api/v1/source-traders/${id}`, {
    refreshMs: 10000,
  });
  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    await client.request(`/api/v1/source-traders/${id}`, {
      method: "PATCH",
      body: JSON.stringify({ alias: data.get("alias"), monitoring_enabled: data.get("monitoring_enabled") === "on" }),
    });
    trader.reload();
  }
  async function remove() {
    await client.request(`/api/v1/source-traders/${id}`, { method: "DELETE" });
    location.href = "/traders";
  }
  if (trader.loading) return <p>Loading…</p>;
  if (trader.error) return <p className="error">{trader.error}</p>;
  if (!trader.data) return <p className="error">Wallet not found.</p>;
  const links: Array<Record<string, unknown>> = Array.isArray(trader.data.links) ? trader.data.links : [];
  const monitoringEnabled = Boolean(trader.data.monitoring_enabled);
  const profile = (trader.data.profile ?? {}) as Record<string, unknown>;
  const profileError = typeof profile.error === "string" && profile.error ? profile.error : null;
  const observedZero = Number(trader.data.observed_trade_count ?? 0) === 0;
  return (
    <section style={{ display: "grid", gap: 16 }}>
      <h1>{String(trader.data.alias)}</h1>
      <div className="panel" style={{ display: "grid", gap: 12 }}>
        <div className="muted">{String(trader.data.wallet_address)}</div>
        <section>
          <h3 style={{ margin: "0 0 6px" }}>Monitoring</h3>
          <p style={{ margin: 0 }}>
            Status: <strong>{monitoringEnabled ? "Enabled" : "Disabled"}</strong>
          </p>
          <p style={{ margin: "4px 0 0" }}>
            Detected trades: <strong>{String(trader.data.observed_trade_count)}</strong>
          </p>
          <p style={{ margin: "4px 0 0" }} className="muted">
            Last detected trade: {String(trader.data.last_trade_at ?? "none")}
          </p>
          {profileError ? (
            <p className="warn" style={{ margin: "8px 0 0" }}>
              Profile sync: {profileError}
            </p>
          ) : observedZero && monitoringEnabled ? (
            <p className="muted" style={{ margin: "8px 0 0" }}>
              Initial trade sync may take a few minutes via the worker.
            </p>
          ) : null}
        </section>
        <section>
          <h3 style={{ margin: "0 0 6px" }}>Copy strategy</h3>
          {links.length === 0 ? (
            <p className="muted" style={{ margin: 0 }}>
              No copy strategy is linked. This wallet is being monitored, but its trades are
              not being evaluated for copying.
            </p>
          ) : (
            <ul style={{ margin: "0 0 8px", paddingLeft: 18 }}>
              {links.map((link) => (
                <li key={String(link.strategy_id)}>
                  {String(link.account_name ?? link.account_id)} — strategy{" "}
                  {Boolean(link.enabled) ? "enabled" : "disabled"}
                </li>
              ))}
            </ul>
          )}
          <p style={{ margin: 0 }}>
            Copied: <strong>{String(trader.data.copied_trade_count)}</strong>
            {" · "}
            Rejected: <strong>{String(trader.data.rejected_trade_count)}</strong>
          </p>
          {links.length === 0 && (
            <p className="muted" style={{ margin: "6px 0 0" }}>
              Copied and Rejected stay at zero until a copy strategy exists and is enabled.
              This does not indicate a worker or API problem.
            </p>
          )}
        </section>
        <section>
          <h3 style={{ margin: "0 0 6px" }}>Trading account</h3>
          {links.length === 0 ? (
            <p className="muted" style={{ margin: 0 }}>
              No trading account linked to a copy strategy. Monitoring does not require a linked account.
            </p>
          ) : (
            <p style={{ margin: 0 }}>
              Linked for copying: {links.map((l) => String(l.account_name ?? l.account_id)).join(", ")}
            </p>
          )}
        </section>
        <button className="ghost" onClick={remove}>Remove wallet</button>
      </div>
      <form className="panel" onSubmit={save}>
        <input name="alias" defaultValue={String(trader.data.alias)} />
        <label><input type="checkbox" name="monitoring_enabled" defaultChecked={monitoringEnabled} /> Monitoring enabled</label>
        <button>Save</button>
      </form>
      <div className="panel">
        <h2>Copy onto an account</h2>
        <p className="muted">Set budget, copy %, risk limits, and preview recent source trades.</p>
        <Link className="button" href={`/traders/${id}/strategy/new`}>Create copy strategy</Link>
      </div>
    </section>
  );
}
