"use client";

import { useEffect, useState } from "react";
import { Desk } from "@/components/desk";
import { useSession } from "@/lib/session";

export default function OverviewPage() {
  return <Desk><Overview /></Desk>;
}

function Overview() {
  const { client } = useSession();
  const [data, setData] = useState<Record<string, unknown> | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    client.request<Record<string, unknown>>("/api/v1/admin/overview").then(setData).catch((err) => setError(err.message));
  }, [client]);
  if (error) return <p className="error">{error}</p>;
  if (!data) return <p>Loading…</p>;
  return (
    <section>
      <h1>Platform overview</h1>
      <div className="grid">
        {Object.entries(data).map(([key, value]) => (
          <article className="card" key={key}><span className="muted">{key}</span><b>{typeof value === "object" ? JSON.stringify(value) : String(value)}</b></article>
        ))}
      </div>
    </section>
  );
}
