"use client";

import Link from "next/link";
import { useParams } from "next/navigation";
import { FormEvent, useState } from "react";
import { Desk, useResource } from "@/components/desk";
import { SourceTraderAnalyticsPanel } from "@/components/source-trader-analytics-panel";
import { SourceTraderCalculatePanel } from "@/components/source-trader-calculate-panel";
import { SimpleFieldLabel } from "@/components/simple-field-label";
import { useToast } from "@/components/toast";
import { useSession } from "@/lib/session";

type TraderTab = "statistics" | "calculate" | "settings";

export default function TraderPage() {
  const params = useParams<{ id: string }>();
  return (
    <Desk>
      <Detail id={params.id} />
    </Desk>
  );
}

function Detail({ id }: { id: string }) {
  const [tab, setTab] = useState<TraderTab>("statistics");
  const { client } = useSession();
  const toast = useToast();
  const trader = useResource<Record<string, unknown>>(`/api/v1/source-traders/${id}`, {
    refreshMs: 30000,
  });
  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    try {
      await client.request(`/api/v1/source-traders/${id}`, {
        method: "PATCH",
        body: JSON.stringify({
          alias: data.get("alias"),
          monitoring_enabled: data.get("monitoring_enabled") === "on",
        }),
      });
      toast.success("Source trader saved");
      trader.reload();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "Save failed");
    }
  }
  async function remove() {
    if (!confirm("Remove this source trader?")) return;
    try {
      await client.request(`/api/v1/source-traders/${id}`, { method: "DELETE" });
      toast.info("Source trader removed");
      location.href = "/traders";
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "Remove failed");
    }
  }
  if (trader.loading) return <p>Loading…</p>;
  if (trader.error) return <p className="error">{trader.error}</p>;
  if (!trader.data) return <p className="error">Wallet not found.</p>;
  const wallet = String(trader.data.wallet_address);
  const alias = String(trader.data.alias);
  return (
    <section style={{ display: "grid", gap: 16, maxWidth: 1100 }}>
      <p style={{ margin: 0 }}>
        <Link href="/traders">← Source traders</Link>
      </p>
      <h1 style={{ margin: 0 }}>{alias}</h1>

      <nav className="filters" aria-label="Source trader sections">
        {(
          [
            ["statistics", "Statistics"],
            ["calculate", "Calculate"],
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

      {tab === "statistics" && (
        <SourceTraderAnalyticsPanel traderId={id} wallet={wallet} alias={alias} />
      )}

      {tab === "calculate" && <SourceTraderCalculatePanel traderId={id} alias={alias} />}

      {tab === "settings" && (
        <div className="panel" style={{ display: "grid", gap: 16 }}>
          <form className="form-stack" onSubmit={save}>
            <div className="form-field">
              <SimpleFieldLabel htmlFor="trader-alias">Display alias</SimpleFieldLabel>
              <input id="trader-alias" name="alias" defaultValue={alias} />
            </div>
            <label>
              <input
                type="checkbox"
                name="monitoring_enabled"
                defaultChecked={Boolean(trader.data.monitoring_enabled)}
              />{" "}
              Monitoring enabled
            </label>
            <button type="submit">Save trader</button>
          </form>
          <div>
            <p className="muted">
              Copied: <strong>{String(trader.data.copied_trade_count)}</strong> · Rejected:{" "}
              <strong>{String(trader.data.rejected_trade_count)}</strong>
            </p>
            <Link className="button" href={`/traders/${id}/strategy/new`}>
              Create copy strategy
            </Link>
          </div>
          <button className="ghost" type="button" onClick={() => void remove()}>
            Remove wallet
          </button>
        </div>
      )}
    </section>
  );
}