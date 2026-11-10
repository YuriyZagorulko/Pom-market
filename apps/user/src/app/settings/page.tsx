"use client";

import { FormEvent, useEffect, useState } from "react";
import { Desk } from "@/components/desk";
import { SimpleFieldLabel } from "@/components/simple-field-label";
import { useToast } from "@/components/toast";
import { useSession } from "@/lib/session";

export default function SettingsPage() {
  return <Desk><Settings /></Desk>;
}

function Settings() {
  const { client, user } = useSession();
  const toast = useToast();
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
    try {
      await client.request("/api/v1/auth/password/change", { method: "POST", body: JSON.stringify(body) });
      toast.success("Password changed");
      event.currentTarget.reset();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "Could not change password");
    }
  }
  return (
    <section style={{ display: "grid", gap: 16 }}>
      <h1>Settings</h1>
      <div className="panel">{user?.email} · {user?.display_name}</div>
      <form className="panel form-stack" onSubmit={changePassword} style={{ maxWidth: 420 }}>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="current_password">Current password</SimpleFieldLabel>
          <input id="current_password" name="current_password" type="password" required autoComplete="current-password" />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="new_password">New password</SimpleFieldLabel>
          <input id="new_password" name="new_password" type="password" minLength={10} required autoComplete="new-password" />
        </div>
        <button type="submit">Change password</button>
      </form>
      <div className="panel">
        <h2>Live updates</h2>
        {events.map((item) => <div key={item}>{item}</div>)}
        {!events.length && <p className="muted">Waiting for activity notifications.</p>}
      </div>
    </section>
  );
}
