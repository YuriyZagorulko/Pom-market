"use client";

import Link from "next/link";
import { Desk, useResource } from "@/components/desk";

export default function TradersPage() {
  return <Desk><List /></Desk>;
}

function List() {
  const { data, error } = useResource<Array<Record<string, unknown>>>("/api/v1/source-traders");
  if (error) return <p className="error">{error}</p>;
  return (
    <section>
      <div className="row"><h1>Source traders</h1><Link className="button" href="/traders/new">Add wallet</Link></div>
      <table>
        <thead><tr><th>Alias</th><th>Address</th><th>Monitoring</th><th>Detected</th><th>Copied</th><th>Rejected</th><th>Last trade</th></tr></thead>
        <tbody>
          {data?.map((trader) => (
            <tr key={String(trader.id)}>
              <td><Link href={`/traders/${trader.id}`}>{String(trader.alias)}</Link></td>
              <td>{String(trader.wallet_address)}</td>
              <td>{trader.monitoring_enabled ? "on" : "off"}</td>
              <td>{String(trader.observed_trade_count)}</td>
              <td>{String(trader.copied_trade_count)}</td>
              <td>{String(trader.rejected_trade_count)}</td>
              <td>{String(trader.last_trade_at ?? "—")}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
