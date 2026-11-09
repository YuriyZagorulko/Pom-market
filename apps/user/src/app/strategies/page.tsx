"use client";

import Link from "next/link";
import { Desk, useResource } from "@/components/desk";

export default function StrategiesPage() {
  return <Desk><List /></Desk>;
}

function List() {
  const { data, error } = useResource<Array<Record<string, string | number | boolean>>>("/api/v1/strategies");
  if (error) return <p className="error">{error}</p>;
  return (
    <section>
      <h1>Copy strategies</h1>
      <p className="muted">Each account and source wallet pair has its own budget. A new strategy starts disabled.</p>
      <table>
        <thead><tr><th>Account</th><th>Trader</th><th>Mode</th><th>Budget</th><th>Enabled</th></tr></thead>
        <tbody>
          {data?.map((row) => (
            <tr key={String(row.id)}>
              <td><Link href={`/strategies/${row.id}`}>{String(row.account_name)}</Link></td>
              <td>{String(row.alias)} · {String(row.wallet_address)}</td>
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
