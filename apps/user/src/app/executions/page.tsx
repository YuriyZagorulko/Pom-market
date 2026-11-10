"use client";

import { PagedListPage } from "@/components/paged-list-page";
import { formatPricePerShare, formatShares, formatUsdc, polygonscanUrl } from "@/lib/source-trade-format";

export default function Page() {
  return (
    <PagedListPage
      title="Executions"
      subtitle="Account fills (not source-wallet trades)"
      apiPath="/api/v1/fills"
      fields={[
        { type: "account", name: "account_id" },
        { type: "text", name: "q", placeholder: "Market, tx, order id" },
        {
          type: "select",
          name: "side",
          options: [
            { value: "", label: "Any side" },
            { value: "buy", label: "Buy" },
            { value: "sell", label: "Sell" },
          ],
        },
        { type: "date", name: "from" },
        { type: "date", name: "to" },
      ]}
      render={({ items }) => (
        <ul className="record-cards">
          {items.map((row) => {
            const tx = polygonscanUrl(row.tx_hash as string);
            return (
              <li key={String(row.id)} className="panel record-card">
                <div className="activity-card-head">
                  {typeof row.market_icon === "string" && row.market_icon ? (
                    // eslint-disable-next-line @next/next/no-img-element
                    <img className="activity-thumb" src={row.market_icon} alt="" />
                  ) : null}
                  <div>
                    <strong>{String(row.market_title ?? "Execution")}</strong>
                    <p className="muted" style={{ margin: "4px 0 0" }}>
                      {row.outcome ? String(row.outcome) : "—"} · {String(row.order_side ?? "—").toUpperCase()} ·{" "}
                      {String(row.filled_at)}
                    </p>
                    <p style={{ margin: 0 }}>
                      {formatShares(row.shares as string)} @ {formatPricePerShare(row.price as string)} ·{" "}
                      {formatUsdc(row.notional_usd as string)}
                    </p>
                    {tx && (
                      <a href={tx} target="_blank" rel="noopener noreferrer">
                        Transaction
                      </a>
                    )}
                  </div>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    />
  );
}
