"use client";

import Link from "next/link";
import { FormEvent, useEffect, useState } from "react";
import { Desk } from "@/components/desk";
import { inviteFeedback, type InviteResult } from "@/lib/invite-feedback";
import { useSession } from "@/lib/session";

type UserRow = { id: string; email: string; display_name: string; role: string; status: string; created_at: string; last_login_at: string | null; accounts: number; traders: number };

export default function UsersPage() {
  return <Desk><Users /></Desk>;
}

function Users() {
  const { client } = useSession();
  const [rows, setRows] = useState<UserRow[]>([]);
  const [invite, setInvite] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [submitting, setSubmitting] = useState(false);
  function load() { client.request<UserRow[]>("/api/v1/admin/users").then(setRows).catch((err) => setError(err.message)); }
  useEffect(load, [client]);
  async function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setError("");
    setNotice("");
    setInvite("");
    const body = Object.fromEntries(new FormData(event.currentTarget).entries());
    setSubmitting(true);
    try {
      const created = await client.request<InviteResult>(
        "/api/v1/admin/users",
        { method: "POST", body: JSON.stringify(body) },
      );
      const feedback = inviteFeedback(created);
      if (feedback.ok) setNotice(feedback.message);
      else setError(feedback.message);
      if (created.setup_url) setInvite(created.setup_url);
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not create user");
    } finally {
      setSubmitting(false);
    }
  }
  return (
    <section>
      <h1>Users</h1>
      <form className="panel filters" onSubmit={create}>
        <input name="email" type="email" required placeholder="Email" />
        <input name="display_name" required placeholder="Display name" />
        <select name="role" defaultValue="user"><option value="user">user</option><option value="admin">admin</option></select>
        <button disabled={submitting}>{submitting ? "Sending…" : "Create invitation"}</button>
      </form>
      {notice && <p className="panel notice success">{notice}</p>}
      {invite && (
        <p className="panel notice">
          <strong>Invitation link</strong> (shown once):<br />
          <code style={{ wordBreak: "break-all" }}>{invite}</code>
        </p>
      )}
      {error && <p className="panel notice error">{error}</p>}
      <table>
        <thead><tr><th>Email</th><th>Name</th><th>Role</th><th>Status</th><th>Accounts</th><th>Wallets</th><th>Last login</th></tr></thead>
        <tbody>
          {rows.map((user) => (
            <tr key={user.id}>
              <td><Link href={`/users/${user.id}`}>{user.email}</Link></td>
              <td>{user.display_name}</td>
              <td>{user.role}</td>
              <td>{user.status}</td>
              <td>{user.accounts}</td>
              <td>{user.traders}</td>
              <td>{user.last_login_at ?? "—"}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
