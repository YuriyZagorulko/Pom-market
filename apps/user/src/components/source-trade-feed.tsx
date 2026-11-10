"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { CopyButton } from "@/components/copy-button";
import { DateRangePresets } from "@/components/date-range-presets";
import { FilterChips } from "@/components/filter-chips";
import { Desk } from "@/components/desk";
import { PaginationBar } from "@/components/pagination-bar";
import { SimpleFieldLabel } from "@/components/simple-field-label";
import { useToast } from "@/components/toast";
import { WalletMultiselect } from "@/components/wallet-multiselect";
import { useListQuery } from "@/lib/use-list-query";
import { usePagedList } from "@/lib/use-paged-list";
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

export function WalletActivityFeed() {
  const toast = useToast();
  const list = useListQuery();
  const traders = usePagedList<{ id: string; alias: string; wallet_address: string }>("/api/v1/source-traders");
  const path = `/api/v1/source-trades?${list.apiParams.toString()}`;
  const { items: rows, total, limit, offset, loading, error, reload } = usePagedList<SourceTrade>(path);
  const [expanded, setExpanded] = useState<string | null>(null);
  const selectedWallets = list.read("wallets") ? list.read("wallets").split(",").filter(Boolean) : [];

  const grouped = useMemo(() => {
    const map = new Map<string, SourceTrade[]>();
    for (const row of rows) {
      const day = row.event_ts.slice(0, 10);
      const bucket = map.get(day) ?? [];
      bucket.push(row);
      map.set(day, bucket);
    }
    return Array.from(map.entries());
  }, [rows]);

  return (
    <section className="wallet-activity">
      <div className="row">
        <div>
          <h1>Wallet activity</h1>
          <p className="muted" style={{ margin: 0 }}>
            Trades from monitored Polymarket wallets
          </p>
        </div>
        <button
          type="button"
          className="ghost"
          onClick={() => {
            reload();
            toast.info("Activity feed refreshed");
          }}
        >
          Refresh
        </button>
      </div>

      <DateRangePresets
        from={list.read("from")}
        to={list.read("to")}
        onChange={(patch) => list.setParams(patch, true)}
      />
      <div className="filters list-filters">
        <WalletMultiselect
          traders={traders.items}
          selected={selectedWallets}
          onChange={(ids) =>
            list.setParams(
              { wallets: ids.length === traders.items.length || ids.length === 0 ? null : ids.join(",") },
              true,
            )
          }
        />
        <div className="form-field">
          <SimpleFieldLabel htmlFor="wa-q">Search</SimpleFieldLabel>
          <input
            id="wa-q"
            defaultValue={list.read("q")}
            onBlur={(e) => list.setParams({ q: e.target.value.trim() || null }, true)}
            onKeyDown={(e) => {
              if (e.key === "Enter") list.setParams({ q: (e.target as HTMLInputElement).value.trim() || null }, true);
            }}
          />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="wa-side">Side</SimpleFieldLabel>
          <select id="wa-side" value={list.read("side")} onChange={(e) => list.setParams({ side: e.target.value || null }, true)}>
            <option value="">Buy and sell</option>
            <option value="buy">Buy</option>
            <option value="sell">Sell</option>
          </select>
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="wa-outcome">Outcome</SimpleFieldLabel>
          <select id="wa-outcome" value={list.read("outcome")} onChange={(e) => list.setParams({ outcome: e.target.value || null }, true)}>
            <option value="">Any outcome</option>
            <option value="Yes">Yes</option>
            <option value="No">No</option>
          </select>
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="wa-type">Activity type</SimpleFieldLabel>
          <select id="wa-type" value={list.read("activity_type")} onChange={(e) => list.setParams({ activity_type: e.target.value || null }, true)}>
            <option value="">Any type</option>
            <option value="trade">Trade</option>
          </select>
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="wa-status">Ingestion status</SimpleFieldLabel>
          <select id="wa-status" value={list.read("status")} onChange={(e) => list.setParams({ status: e.target.value || null }, true)}>
            <option value="">Any status</option>
            <option value="processed">Processed</option>
            <option value="detected">Detected</option>
            <option value="normalized">Normalized</option>
            <option value="processing">Processing</option>
            <option value="failed">Failed</option>
            <option value="duplicate">Duplicate</option>
          </select>
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="wa-from">From date</SimpleFieldLabel>
          <input id="wa-from" type="date" defaultValue={list.read("from")} onBlur={(e) => list.setParams({ from: e.target.value || null }, true)} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="wa-to">To date</SimpleFieldLabel>
          <input id="wa-to" type="date" defaultValue={list.read("to")} onBlur={(e) => list.setParams({ to: e.target.value || null }, true)} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="wa-min-amt">Min total USDC</SimpleFieldLabel>
          <input id="wa-min-amt" defaultValue={list.read("min_amount")} onBlur={(e) => list.setParams({ min_amount: e.target.value.trim() || null }, true)} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="wa-max-amt">Max total USDC</SimpleFieldLabel>
          <input id="wa-max-amt" defaultValue={list.read("max_amount")} onBlur={(e) => list.setParams({ max_amount: e.target.value.trim() || null }, true)} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="wa-min-price">Min price</SimpleFieldLabel>
          <input id="wa-min-price" defaultValue={list.read("min_price")} onBlur={(e) => list.setParams({ min_price: e.target.value.trim() || null }, true)} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="wa-max-price">Max price</SimpleFieldLabel>
          <input id="wa-max-price" defaultValue={list.read("max_price")} onBlur={(e) => list.setParams({ max_price: e.target.value.trim() || null }, true)} />
        </div>
      </div>

      <FilterChips
        chips={list.chips}
        onRemove={(name) => list.setParams({ [name]: null }, true)}
        onClear={() => list.clearAll()}
      />

      {loading && <p className="muted">Loading…</p>}
      {error && <p className="error">{error}</p>}
      {!loading && !error && !rows.length && (
        <div className="activity-empty panel">
          <p style={{ margin: 0 }}>No wallet trades yet.</p>
          <p className="muted" style={{ margin: "8px 0 0" }}>
            Add a source wallet under Source traders and enable monitoring.
          </p>
        </div>
      )}

      {grouped.map(([day, bucket]) => (
        <section key={day} className="activity-day-group">
          <h2 className="activity-day-heading">{day}</h2>
          <ul className="activity-feed">
            {bucket.map((trade) => (
              <ActivityCard
                key={trade.id}
                trade={trade}
                open={expanded === trade.id}
                onToggle={() => setExpanded((cur) => (cur === trade.id ? null : trade.id))}
              />
            ))}
          </ul>
        </section>
      ))}

      {!loading && (
        <PaginationBar
          total={total}
          limit={limit}
          offset={offset}
          page={list.page}
          pageSizeOptions={list.pageSizeOptions}
          onPage={(p) => list.setParams({ page: String(p) })}
          onLimit={(n) => list.setParams({ limit: String(n), page: "1" })}
        />
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
