"use client";

import { PagedListPage } from "@/components/paged-list-page";
import { formatPricePerShare, formatShares, formatUsdc } from "@/lib/source-trade-format";

export default function Page() {
  return (
    <PagedListPage
      title="Positions"
      subtitle="Open paper/live positions per account"
      apiPath="/api/v1/positions"
      fields={[
        { type: "account", name: "account_id" },
        { type: "text", name: "q", placeholder: "Market, outcome, token" },
      ]}
      render={({ items }) => (
        <ul className="record-cards">
          {items.map((row) => (
            <li key={String(row.id)} className="panel record-card">
              <strong>{String(row.market_title ?? row.token_id)}</strong>
              <p className="muted" style={{ margin: "4px 0" }}>
                {row.outcome ? String(row.outcome) : "—"} · {String(row.operating_mode)}
              </p>
              <p style={{ margin: 0 }}>
                {formatShares(row.open_shares as string)} @ avg {formatPricePerShare(row.avg_price as string)}
              </p>
              <p className="muted" style={{ margin: "4px 0 0" }}>
                Cost {formatUsdc(row.cost_basis_usd as string)} · Realized PnL {formatUsdc(row.realized_pnl_usd as string)}
              </p>
            </li>
          ))}
        </ul>
      )}
    />
  );
}
