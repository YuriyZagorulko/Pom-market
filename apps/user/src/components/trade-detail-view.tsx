"use client";

import Link from "next/link";
import { CopyButton } from "@/components/copy-button";
import {
  activityLabel,
  formatEventTime,
  formatPricePerShare,
  formatShares,
  formatUsdc,
  polygonscanUrl,
  polymarketUrl,
  shortenAddress,
  type SourceTrade,
} from "@/lib/source-trade-format";

type TradeBundle = {
  trade: SourceTrade;
  links?: { polygonscan?: string | null; polymarket?: string | null };
  decisions?: unknown[];
  orders?: unknown[];
  fills?: unknown[];
  timeline?: Array<{ id: string; kind: string; message: string; at: string }>;
};

export function TradeDetailView({ data }: { data: TradeBundle }) {
  const trade = data.trade;
  const side = trade.side?.toLowerCase() === "sell" ? "sell" : "buy";
  const title = trade.market_title?.trim() || trade.event_title?.trim() || "Unknown market";
  const marketUrl = data.links?.polymarket ?? polymarketUrl(trade);
  const txUrl = data.links?.polygonscan ?? polygonscanUrl(trade.tx_hash);
  const usdc = trade.usdc_size != null && trade.usdc_size !== "" ? formatUsdc(trade.usdc_size) : null;
  const notional = formatUsdc(trade.notional_usd);
  const hasRelated =
    (data.decisions?.length ?? 0) > 0 ||
    (data.orders?.length ?? 0) > 0 ||
    (data.fills?.length ?? 0) > 0 ||
    (data.timeline?.length ?? 0) > 0;

  return (
    <section className="trade-detail">
      <div className={`trade-detail-hero panel side-${side}`}>
        <div className="activity-card-head">
          <MarketThumb icon={trade.market_icon} title={title} />
          <div>
            <div className="activity-card-title-row">
              <span className="activity-type">{activityLabel(trade.activity_type)}</span>
              <span className={`side-pill ${side}`}>{side.toUpperCase()}</span>
              <time className="muted">{formatEventTime(trade.event_ts)}</time>
            </div>
            <h1 className="activity-market-title">{title}</h1>
            <p className="muted" style={{ margin: "6px 0 0" }}>
              {shortenAddress(trade.source_wallet)} · {trade.status ?? "—"}
            </p>
          </div>
        </div>
      </div>

      <div className="grid">
        <Stat label="Price" value={formatPricePerShare(trade.price)} />
        <Stat label="Shares" value={formatShares(trade.size_shares)} />
        <Stat label="USDC size" value={usdc ?? "—"} />
        <Stat label="Notional" value={notional ?? "—"} />
      </div>

      <section className="panel">
        <h2>Market</h2>
        <p>{trade.outcome ? `Outcome: ${trade.outcome}` : "Outcome: —"}</p>
        {trade.market_slug && <p className="muted">Market slug: {trade.market_slug}</p>}
        {trade.event_slug && <p className="muted">Event slug: {trade.event_slug}</p>}
        <div className="filters">
          {marketUrl && (
            <a className="button" href={marketUrl} target="_blank" rel="noopener noreferrer">
              Open on Polymarket
            </a>
          )}
          {txUrl && (
            <a className="button ghost" href={txUrl} target="_blank" rel="noopener noreferrer">
              Polygonscan
            </a>
          )}
          <Link className="ghost button" href="/trades">
            Back to activity
          </Link>
        </div>
      </section>

      <section className="panel">
        <h2>Transaction</h2>
        <DetailRow label="Wallet" value={trade.source_wallet} copy />
        {trade.tx_hash && <DetailRow label="Tx hash" value={trade.tx_hash} copy />}
        <DetailRow label="Token ID" value={trade.token_id} copy />
        {trade.condition_id && <DetailRow label="Condition ID" value={trade.condition_id} copy />}
        <DetailRow label="Timestamp" value={trade.event_ts} />
        {trade.origin && <DetailRow label="Origin" value={trade.origin} />}
      </section>

      <section className="panel">
        <h2>Related account activity</h2>
        {!hasRelated ? (
          <p className="muted">No related account activity is available for this source trade.</p>
        ) : (
          <>
            {!!data.timeline?.length && (
              <div className="timeline">
                {data.timeline.map((item) => (
                  <div key={item.id}>
                    <b>{formatEventTime(item.at)}</b> — {item.kind}: {item.message}
                  </div>
                ))}
              </div>
            )}
            {!!data.decisions?.length && <p>Decisions: {data.decisions.length}</p>}
            {!!data.orders?.length && <p>Orders: {data.orders.length}</p>}
            {!!data.fills?.length && <p>Fills: {data.fills.length}</p>}
          </>
        )}
      </section>
    </section>
  );
}

function Stat({ label, value }: { label: string; value: string | null }) {
  return (
    <article className="card">
      <span className="muted">{label}</span>
      <b>{value ?? "—"}</b>
    </article>
  );
}

function MarketThumb({ icon, title }: { icon?: string | null; title: string }) {
  return (
    <div className="activity-thumb" aria-hidden>
      {icon ? (
        // eslint-disable-next-line @next/next/no-img-element
        <img src={icon} alt="" loading="lazy" />
      ) : (
        <span>{title.slice(0, 1).toUpperCase()}</span>
      )}
    </div>
  );
}

function DetailRow({ label, value, copy }: { label: string; value: string; copy?: boolean }) {
  return (
    <div className="activity-detail-row">
      <span className="muted">{label}</span>
      <code>{value}</code>
      {copy && <CopyButton value={value} label="Copy" />}
    </div>
  );
}
