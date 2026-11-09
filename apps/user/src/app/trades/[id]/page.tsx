"use client";

import { useParams } from "next/navigation";
import { Desk, useResource } from "@/components/desk";

export default function TradePage() {
  const params = useParams<{ id: string }>();
  return <Desk><Detail id={params.id} /></Desk>;
}

function Detail({ id }: { id: string }) {
  const { data, error } = useResource<Record<string, unknown>>(`/api/v1/trades/${id}`);
  if (error) return <p className="error">{error}</p>;
  if (!data) return <p>Loading…</p>;
  const trade = data.trade as Record<string, string>;
  const timeline = (data.timeline as Array<Record<string, string>>) ?? [];
  const links = data.links as Record<string, string | null>;
  return (
    <section>
      <h1>{trade.side.toUpperCase()} {trade.market_title || trade.token_id}</h1>
      <p className="muted">{trade.source_wallet} · {trade.event_ts} · {trade.origin}</p>
      <div className="grid">
        <article className="card"><span className="muted">Price</span><b>{trade.price}</b></article>
        <article className="card"><span className="muted">Shares</span><b>{trade.size_shares}</b></article>
        <article className="card"><span className="muted">Notional</span><b>{trade.notional_usd}</b></article>
        <article className="card"><span className="muted">Status</span><b>{trade.status}</b></article>
      </div>
      <div className="filters">
        {links.polygonscan && <a className="button" href={links.polygonscan}>Polygonscan</a>}
        {links.polymarket && <a className="button" href={links.polymarket}>Market</a>}
      </div>
      <div className="panel timeline">
        {timeline.map((item) => <div key={item.id}><b>{item.at}</b> — {item.kind}: {item.message}</div>)}
        {!timeline.length && <div>No account-level timeline yet. The source trade is stored above.</div>}
      </div>
      <h2>Decisions</h2>
      <pre>{JSON.stringify(data.decisions, null, 2)}</pre>
      <h2>Orders</h2>
      <pre>{JSON.stringify(data.orders, null, 2)}</pre>
      <h2>Fills</h2>
      <pre>{JSON.stringify(data.fills, null, 2)}</pre>
    </section>
  );
}
