"use client";

import { useEffect, useState } from "react";
import { Desk } from "@/components/desk";
import { useSession } from "@/lib/session";

export default function HealthPage() {
  return <Desk><Health /></Desk>;
}

function Health() {
  const { client } = useSession();
  const [rows, setRows] = useState<Array<Record<string, unknown>>>([]);
  useEffect(() => { client.request<Array<Record<string, unknown>>>("/api/v1/admin/health").then(setRows); }, [client]);
  return (
    <section>
      <h1>System health</h1>
      {rows.map((row) => (
        <article className="panel" key={String(row.key)}>
          <b>{String(row.key)}</b>
          <div className="muted">{String(row.updated_at)}</div>
          <pre>{JSON.stringify(row.value, null, 2)}</pre>
        </article>
      ))}
    </section>
  );
}
