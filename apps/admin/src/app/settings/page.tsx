"use client";

import { useEffect, useState } from "react";
import { Desk } from "@/components/desk";
import { useSession } from "@/lib/session";

export default function SettingsPage() {
  return <Desk><Settings /></Desk>;
}

function Settings() {
  const { client } = useSession();
  const [enabled, setEnabled] = useState(false);
  useEffect(() => {
    client.request<Record<string, { enabled?: boolean }>>("/api/v1/admin/overview").then((data) => {
      const kill = data.kill_switch as { enabled?: boolean } | undefined;
      setEnabled(Boolean(kill?.enabled));
    });
  }, [client]);
  return (
    <section>
      <h1>Settings</h1>
      <div className="panel">
        <p>The global kill switch stops new live orders. It does not delete history, positions, or strategies. Paper and monitoring continue.</p>
        <button onClick={() => client.request("/api/v1/admin/kill-switch", { method: "POST", body: JSON.stringify({ enabled: !enabled }) }).then(() => setEnabled(!enabled))}>
          {enabled ? "Kill switch is ON — click to release" : "Turn kill switch ON"}
        </button>
      </div>
    </section>
  );
}
