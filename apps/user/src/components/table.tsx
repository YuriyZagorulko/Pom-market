"use client";

import Link from "next/link";
import { useState } from "react";
import { Desk, useResource } from "@/components/desk";
import { SimpleFieldLabel } from "@/components/simple-field-label";
import { useToast } from "@/components/toast";

export function TablePage({ title, path, linkPrefix }: { title: string; path: string; linkPrefix?: string }) {
  return <Desk><Table title={title} path={path} linkPrefix={linkPrefix} /></Desk>;
}

function Table({ title, path, linkPrefix }: { title: string; path: string; linkPrefix?: string }) {
  const toast = useToast();
  const [query, setQuery] = useState("");
  const [side, setSide] = useState("");
  const [status, setStatus] = useState("");
  const params = new URLSearchParams({ limit: "50" });
  if (query) params.set("q", query);
  if (side) params.set("side", side);
  if (status) params.set("status", status);
  const { data, error, reload } = useResource<Array<Record<string, unknown>>>(`${path}?${params.toString()}`);
  const rows = data ?? [];
  const columns = rows[0] ? Object.keys(rows[0]).slice(0, 8) : [];
  return (
    <section>
      <div className="row">
        <h1>{title}</h1>
        <button
          type="button"
          className="ghost"
          onClick={() => {
            reload();
            toast.info("List refreshed");
          }}
        >
          Refresh
        </button>
      </div>
      <div className="filters list-filters">
        <div className="form-field">
          <SimpleFieldLabel htmlFor="table-q">Search</SimpleFieldLabel>
          <input id="table-q" value={query} onChange={(event) => setQuery(event.target.value)} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="table-side">Side</SimpleFieldLabel>
          <select id="table-side" value={side} onChange={(event) => setSide(event.target.value)}>
            <option value="">Buy and sell</option>
            <option value="buy">Buy</option>
            <option value="sell">Sell</option>
          </select>
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="table-status">Status</SimpleFieldLabel>
          <input id="table-status" value={status} onChange={(event) => setStatus(event.target.value)} />
        </div>
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
