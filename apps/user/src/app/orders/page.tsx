"use client";

import { PagedListPage } from "@/components/paged-list-page";
import { formatPricePerShare, formatShares } from "@/lib/source-trade-format";

export default function Page() {
  return (
    <PagedListPage
      title="Orders"
      subtitle="Your copy-trading orders"
      apiPath="/api/v1/orders"
      fields={[
        { type: "account", name: "account_id" },
        { type: "text", name: "q", placeholder: "Market or order id" },
        {
          type: "select",
          name: "side",
          options: [
            { value: "", label: "Any side" },
            { value: "buy", label: "Buy" },
            { value: "sell", label: "Sell" },
          ],
        },
        {
          type: "select",
          name: "status",
          options: [
            { value: "", label: "Any status" },
            { value: "filled", label: "Filled" },
            { value: "pending", label: "Pending" },
            { value: "failed", label: "Failed" },
          ],
        },
        { type: "date", name: "from" },
        { type: "date", name: "to" },
      ]}
      render={({ items }) => (
        <ul className="record-cards">
          {items.map((row) => (
            <li key={String(row.id)} className="panel record-card">
              <div className="row">
                <strong>{String(row.market_title ?? row.token_id)}</strong>
                <span className={`side-pill ${String(row.side)}`}>{String(row.side).toUpperCase()}</span>
              </div>
              <p className="muted" style={{ margin: "4px 0" }}>
                {row.outcome ? String(row.outcome) : "—"} · {String(row.status)} · {String(row.order_type)}
              </p>
              <p style={{ margin: 0 }}>
                {formatShares(row.size_shares as string)} @ {formatPricePerShare(row.limit_price as string)} ·{" "}
                {String(row.created_at)}
              </p>
            </li>
          ))}
        </ul>
      )}
    />
  );
}
