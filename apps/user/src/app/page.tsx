"use client";

import Link from "next/link";
import { Desk, useResource } from "@/components/desk";

type Summary = {
  accounts: number;
  source_traders: number;
  detected_trades: number;
  copied_trades: number;
  skipped_or_rejected: number;
  open_positions: number;
  realized_pnl_usd: string;
  account_cards: Array<Record<string, string | null>>;
  collector: Record<string, string>;
  worker: { at?: string };
};

export default function DashboardPage() {
  return <Desk><Dashboard /></Desk>;
}

function Dashboard() {
  const { data, error } = useResource<Summary>("/api/v1/dashboard");
  if (error) return <p className="error">{error}</p>;
  if (!data) return <p>Loading…</p>;
  if (data.accounts === 0) {
    return (
      <section className="panel">
        <h1>No Polymarket account yet</h1>
        <p>Add the wallet you want copy trades to land on. New accounts stay in monitoring until you choose otherwise.</p>
        <Link className="button" href="/accounts/new">Add Polymarket account</Link>
      </section>
    );
  }
  const cards = [
    ["Accounts", data.accounts],
    ["Source wallets", data.source_traders],
    ["Detected", data.detected_trades],
    ["Copied", data.copied_trades],
    ["Skipped / rejected", data.skipped_or_rejected],
    ["Open positions", data.open_positions],
    ["Realized PnL", data.realized_pnl_usd],
  ];
  return (
    <section>
      <h1>Dashboard</h1>
      <p className="muted">Worker heartbeat: {data.worker?.at || "not reported"} · Collector: {JSON.stringify(data.collector)}</p>
      <div className="grid">
        {cards.map(([label, value]) => <article className="card" key={String(label)}><span className="muted">{label}</span><b>{String(value)}</b></article>)}
      </div>
      <div className="panel">
        <h2>Accounts</h2>
        <table>
          <thead><tr><th>Name</th><th>Funder</th><th>Mode</th><th>Status</th><th>Exchange balance</th></tr></thead>
          <tbody>
            {data.account_cards.map((account) => (
              <tr key={String(account.id)}>
                <td><Link href={`/accounts/${account.id}`}>{account.name}</Link></td>
                <td>{account.funder_address}</td>
                <td>{account.operating_mode}</td>
                <td>{account.status}</td>
                <td>{account.last_exchange_balance_usd ?? "not checked"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}
