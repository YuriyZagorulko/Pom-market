"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { Desk } from "@/components/desk";
import {
  countMetrics,
  type OverviewResponse,
  parseKillSwitch,
} from "@/lib/overview-metrics";
import { useSession } from "@/lib/session";

export default function OverviewPage() {
  return (
    <Desk>
      <Overview />
    </Desk>
  );
}

function Overview() {
  const { client } = useSession();
  const [data, setData] = useState<OverviewResponse | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    client
      .request<OverviewResponse>("/api/v1/admin/overview")
      .then(setData)
      .catch((err) => setError(err.message));
  }, [client]);
  if (error) return <p className="error">{error}</p>;
  if (!data) return <p>Loading…</p>;

  const kill = parseKillSwitch(data.kill_switch);
  const metrics = countMetrics(data);

  return (
    <section>
      <h1>Platform overview</h1>
      <div className="grid">
        {metrics.map(({ key, label, value }) => (
          <article className="card" key={key}>
            <span className="muted">{label}</span>
            <strong className="metric-value">{value}</strong>
          </article>
        ))}
        <article className="card">
          <span className="muted">Kill Switch</span>
          <strong
            className={`metric-value kill-badge${kill ? " kill-on" : " kill-off"}`}
          >
            {kill === null ? "Unknown" : kill ? "ON" : "OFF"}
          </strong>
          <p className="muted" style={{ marginTop: 8, fontSize: 12 }}>
            <Link href="/settings">Change in Settings</Link>
          </p>
        </article>
      </div>
    </section>
  );
}
