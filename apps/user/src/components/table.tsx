"use client";

import Link from "next/link";
import { useState } from "react";
import { Desk, useResource } from "@/components/desk";

export function TablePage({ title, path, linkPrefix }: { title: string; path: string; linkPrefix?: string }) {
  return <Desk><Table title={title} path={path} linkPrefix={linkPrefix} /></Desk>;
}

function Table({ title, path, linkPrefix }: { title: string; path: string; linkPrefix?: string }) {
  const [query, setQuery] = useState("");
  const [side, setSide] = useState("");
  const [status, setStatus] = useState("");
  const { data, error, reload } = useResource<Array<Record<string, unknown>>>(`${path}?limit=50&q=${encodeURIComponent(query)}&side=${side}&status=${status}`);
  const rows = data ?? [];
  const columns = rows[0] ? Object.keys(rows[0]).slice(0, 8) : [];
  return (
    <section>
      <div className="row"><h1>{title}</h1><button className="ghost" onClick={reload}>Refresh</button></div>
      <div className="filters">
        <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search address, market, token, tx" />
        <select value={side} onChange={(event) => setSide(event.target.value)}><option value="">Buy and sell</option><option value="buy">Buy</option><option value="sell">Sell</option></select>
        <input value={status} onChange={(event) => setStatus(event.target.value)} placeholder="Status" />
      </div>
      {error && <p className="error">{error}</p>}
      {!rows.length && <p className="muted">No rows yet.</p>}
      {!!rows.length && (
        <table>
          <thead><tr>{columns.map((column) => <th key={column}>{column}</th>)}</tr></thead>
          <tbody>
            {rows.map((row) => (
              <tr key={String(row.id)}>
                {columns.map((column) => (
                  <td key={column}>
                    {column === "id" && linkPrefix ? <Link href={`${linkPrefix}/${row.id}`}>{String(row[column])}</Link> : String(row[column] ?? "")}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
