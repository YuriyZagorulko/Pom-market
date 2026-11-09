"use client";

import { Desk, useResource } from "@/components/desk";

export default function PositionsPage() {
  return <Desk><Positions /></Desk>;
}

function Positions() {
  const open = useResource<Array<Record<string, string>>>("/api/v1/positions");
  const closed = useResource<Array<Record<string, string>>>("/api/v1/closed-positions");
  return (
    <section>
      <h1>Positions</h1>
      <h2>Open</h2>
      <Simple rows={open.data} error={open.error} />
      <h2>Closed</h2>
      <Simple rows={closed.data} error={closed.error} />
    </section>
  );
}

function Simple({ rows, error }: { rows: Array<Record<string, string>> | null; error: string }) {
  if (error) return <p className="error">{error}</p>;
  if (!rows?.length) return <p className="muted">None.</p>;
  const columns = Object.keys(rows[0]).slice(0, 8);
  return (
    <table>
      <thead><tr>{columns.map((column) => <th key={column}>{column}</th>)}</tr></thead>
      <tbody>{rows.map((row) => <tr key={row.id}>{columns.map((column) => <td key={column}>{String(row[column] ?? "")}</td>)}</tr>)}</tbody>
    </table>
  );
}
