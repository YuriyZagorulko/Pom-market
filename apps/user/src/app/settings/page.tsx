"use client";

import { FormEvent, useEffect, useState } from "react";
import { Desk } from "@/components/desk";
import { useSession } from "@/lib/session";

export default function SettingsPage() {
  return <Desk><Settings /></Desk>;
}

function Settings() {
  const { client, user } = useSession();
  const [message, setMessage] = useState("");
  const [events, setEvents] = useState<string[]>([]);
  useEffect(() => {
    client.refresh().then((result) => {
      const socket = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.hostname}:4321/api/v1/ws?token=${encodeURIComponent(result.access_token)}`);
      socket.onmessage = (event) => setEvents((items) => [event.data, ...items].slice(0, 8));
      return () => socket.close();
    }).catch(() => undefined);
  }, [client]);
  async function changePassword(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const body = Object.fromEntries(new FormData(event.currentTarget).entries());
    await client.request("/api/v1/auth/password/change", { method: "POST", body: JSON.stringify(body) });
    setMessage("Password changed");
  }
  return (
    <section style={{ display: "grid", gap: 16 }}>
      <h1>Settings</h1>
      <div className="panel">{user?.email} · {user?.display_name}</div>
      <form className="panel" onSubmit={changePassword} style={{ display: "grid", gap: 8, maxWidth: 420 }}>
        <input name="current_password" type="password" placeholder="Current password" required />
        <input name="new_password" type="password" placeholder="New password" minLength={10} required />
        <button>Change password</button>
        {message && <div className="ok">{message}</div>}
      </form>
      <div className="panel">
        <h2>Live updates</h2>
        {events.map((item) => <div key={item}>{item}</div>)}
        {!events.length && <p className="muted">Waiting for activity notifications.</p>}
      </div>
    </section>
  );
}
