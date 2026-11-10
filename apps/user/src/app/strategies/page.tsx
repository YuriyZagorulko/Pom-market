"use client";

import Link from "next/link";
import { Desk, useResource } from "@/components/desk";
import { StrategiesListHeader } from "@/components/strategies-list-header";

export default function StrategiesPage() {
  return (
    <Desk>
      <List />
    </Desk>
  );
}

function List() {
  const { data, error, loading } = useResource<Array<Record<string, string | number | boolean>>>("/api/v1/strategies");
  if (error) return <p className="error">{error}</p>;
  const rows = data ?? [];
  return (
    <section>
      <StrategiesListHeader />
      {loading && rows.length === 0 ? <p className="muted">Loading…</p> : null}
      {!loading && rows.length === 0 ? (
        <div className="panel">
          <p className="muted" style={{ margin: 0 }}>
            No copy strategies yet. You need a paper trading account and a monitored source wallet, then{" "}
            <Link href="/strategies/new">create a strategy</Link>.
          </p>
        </div>
      ) : null}
      <table>
        <thead>
          <tr>
            <th>Account</th>
            <th>Trader</th>
            <th>Mode</th>
            <th>Budget</th>
            <th>Enabled</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={String(row.id)}>
              <td>
                <Link href={`/strategies/${row.id}`}>{String(row.account_name)}</Link>
              </td>
              <td>
                {String(row.alias)} · {String(row.wallet_address)}
              </td>
              <td>{String(row.sizing_mode)}</td>
              <td>{String(row.strategy_budget_usd)}</td>
              <td>{row.enabled ? "yes" : "no"}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
