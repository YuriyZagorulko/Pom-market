"use client";

import { useEffect, useState } from "react";
import { Desk } from "@/components/desk";
import { useSession } from "@/lib/session";

export default function EventsPage() {
  return <Desk><Events /></Desk>;
}

function Events() {
  const { client } = useSession();
  const [rows, setRows] = useState<Array<Record<string, string>>>([]);
  useEffect(() => { client.request<Array<Record<string, string>>>("/api/v1/admin/events").then(setRows); }, [client]);
  return (
    <section>
      <h1>System events</h1>
      <table>
        <thead><tr><th>When</th><th>Severity</th><th>Component</th><th>Message</th></tr></thead>
        <tbody>{rows.map((row) => <tr key={row.id}><td>{row.created_at}</td><td>{row.severity}</td><td>{row.component}</td><td>{row.message}</td></tr>)}</tbody>
      </table>
      {!rows.length && <p className="muted">No system events recorded.</p>}
    </section>
  );
}
