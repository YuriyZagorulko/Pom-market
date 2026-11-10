"use client";

import Link from "next/link";
import { useCallback, useMemo, useState } from "react";
import { CopyButton } from "@/components/copy-button";
import { Desk, useResource } from "@/components/desk";
import {
  activityLabel,
  displayNotional,
  formatEventTime,
  formatPricePerShare,
  formatShares,
  polygonscanUrl,
  polymarketUrl,
  shortenAddress,
  shortenHash,
  type SourceTrade,
} from "@/lib/source-trade-format";

const PAGE = 50;

export function WalletActivityPage() {
  return (
    <Desk>
      <WalletActivityFeed />
    </Desk>
  );
}

export function WalletActivityFeed() {
  const [query, setQuery] = useState("");
  const [side, setSide] = useState("");
  const [offset, setOffset] = useState(0);
  const [expanded, setExpanded] = useState<string | null>(null);

  const path = useMemo(() => {
    const params = new URLSearchParams({ limit: String(PAGE), offset: String(offset) });
    if (query.trim()) params.set("q", query.trim());
    if (side) params.set("side", side);
    return `/api/v1/source-trades?${params.toString()}`;
  }, [query, side, offset]);

  const { data, error, reload } = useResource<SourceTrade[]>(path);

  const onFilterChange = useCallback(() => setOffset(0), []);

  const rows = data ?? [];
  const canLoadMore = rows.length === PAGE;

  return (
    <section className="wallet-activity">
      <div className="row">
        <div>
          <h1>Wallet activity</h1>
          <p className="muted" style={{ margin: 0 }}>
            Trades from monitored Polymarket wallets
          </p>
        </div>
        <button type="button" className="ghost" onClick={reload}>
          Refresh
        </button>
      </div>

      <div className="filters">
        <input
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            onFilterChange();
          }}
          placeholder="Search wallet, market, token, tx"
        />
        <select
          value={side}
          onChange={(e) => {
            setSide(e.target.value);
            onFilterChange();
          }}
        >
          <option value="">Buy and sell</option>
          <option value="buy">Buy</option>
          <option value="sell">Sell</option>
        </select>
      </div>

      {error && <p className="error">{error}</p>}
      {!error && !rows.length && (
        <div className="activity-empty panel">
          <p style={{ margin: 0 }}>No wallet trades yet.</p>
          <p className="muted" style={{ margin: "8px 0 0" }}>
            Add a source wallet under Source traders and enable monitoring.
          </p>
        </div>
      )}

      <ul className="activity-feed">
        {rows.map((trade) => (
          <ActivityCard
            key={trade.id}
            trade={trade}
            open={expanded === trade.id}
            onToggle={() => setExpanded((cur) => (cur === trade.id ? null : trade.id))}
          />
        ))}
      </ul>

      {!!rows.length && (
        <div className="row activity-pager">
          <button
            type="button"
            className="ghost"
            disabled={offset === 0}
            onClick={() => setOffset((o) => Math.max(0, o - PAGE))}
          >
            Newer
          </button>
          <span className="muted">
            Showing {offset + 1}–{offset + rows.length}
          </span>
          <button
            type="button"
            className="ghost"
            disabled={!canLoadMore}
            onClick={() => setOffset((o) => o + PAGE)}
          >
            Older
          </button>
        </div>
      )}
    </section>
  );
}

function ActivityCard({
  trade,
  open,
  onToggle,
}: {
  trade: SourceTrade;
  open: boolean;
  onToggle: () => void;
}) {
  const side = trade.side?.toLowerCase() === "sell" ? "sell" : "buy";
  const marketUrl = polymarketUrl(trade);
  const txUrl = polygonscanUrl(trade.tx_hash);
  const shares = formatShares(trade.size_shares);
  const price = formatPricePerShare(trade.price);
  const total = displayNotional(trade);
  const title = trade.market_title?.trim() || trade.event_title?.trim() || "Unknown market";
  const outcome = trade.outcome?.trim() || null;

  return (
    <li className={`activity-card panel side-${side}`}>
      <div className="activity-card-head">
        <MarketThumb icon={trade.market_icon} title={title} />
        <div className="activity-card-meta">
          <div className="activity-card-title-row">
            <span className="activity-type">{activityLabel(trade.activity_type)}</span>
            <span className={`side-pill ${side}`}>{side.toUpperCase()}</span>
            <time className="muted">{formatEventTime(trade.event_ts)}</time>
          </div>
          <div className="activity-market-title">{title}</div>
        </div>
      </div>

      <TradeAmounts outcome={outcome} shares={shares} price={price} total={total} />

      <div className="activity-card-foot">
        <span className="muted" title={trade.source_wallet}>
          {shortenAddress(trade.source_wallet)}
        </span>
        <div className="activity-links">
          {marketUrl && (
            <a href={marketUrl} target="_blank" rel="noopener noreferrer">
              Market
            </a>
          )}
          {txUrl && (
            <a href={txUrl} target="_blank" rel="noopener noreferrer">
              Tx {trade.tx_hash ? shortenHash(trade.tx_hash) : ""}
            </a>
          )}
          <Link href={`/trades/${trade.id}`}>Details</Link>
          <button type="button" className="ghost activity-expand" onClick={onToggle}>
            {open ? "Hide" : "More"}
          </button>
        </div>
      </div>

      {open && (
        <div className="activity-details">
          <DetailRow label="Wallet" value={trade.source_wallet} copy />
          {trade.tx_hash && <DetailRow label="Transaction" value={trade.tx_hash} copy />}
          <DetailRow label="Token ID" value={trade.token_id} copy />
          {trade.condition_id && <DetailRow label="Condition ID" value={trade.condition_id} copy />}
          <DetailRow label="Timestamp" value={trade.event_ts} />
          {trade.market_slug && <DetailRow label="Market slug" value={trade.market_slug} />}
          {trade.event_slug && <DetailRow label="Event slug" value={trade.event_slug} />}
          {trade.origin && <DetailRow label="Origin" value={trade.origin} />}
          {trade.status && <DetailRow label="Status" value={trade.status} />}
        </div>
      )}
    </li>
  );
}

function TradeAmounts({
  outcome,
  shares,
  price,
  total,
}: {
  outcome: string | null;
  shares: string | null;
  price: string | null;
  total: string | null;
}) {
  return (
    <div className="activity-card-body">
      <div>
        <span className="muted">Outcome</span>
        <strong>{outcome ?? "—"}</strong>
      </div>
      <div>
        <span className="muted">Shares</span>
        <strong>{shares ?? "—"}</strong>
      </div>
      <div>
        <span className="muted">Price</span>
        <strong>{price ?? "—"}</strong>
      </div>
      <div>
        <span className="muted">Total</span>
        <strong>{total ?? "—"}</strong>
      </div>
    </div>
  );
}

function MarketThumb({ icon, title }: { icon?: string | null; title: string }) {
  const [failed, setFailed] = useState(false);
  const showImg = icon && !failed;
  return (
    <div className="activity-thumb" aria-hidden>
      {showImg ? (
        // eslint-disable-next-line @next/next/no-img-element
        <img src={icon} alt="" loading="lazy" onError={() => setFailed(true)} />
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
