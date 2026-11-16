"use client";

import Link from "next/link";
import { FormEvent, useEffect, useMemo, useState } from "react";
import { Desk } from "@/components/desk";
import { inviteFeedback, type InviteResult } from "@/lib/invite-feedback";
import { useSession } from "@/lib/session";

type UserRow = {
  id: string;
  email: string;
  display_name: string;
  role: string;
  status: string;
  created_at: string;
  updated_at: string;
  last_login_at: string | null;
  accounts: number;
  traders: number;
};

export default function UsersPage() {
  return (
    <Desk>
      <Users />
    </Desk>
  );
}

function formatWhen(iso: string | null) {
  if (!iso) return "—";
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

function Users() {
  const { client } = useSession();
  const [rows, setRows] = useState<UserRow[]>([]);
  const [search, setSearch] = useState("");
  const [invite, setInvite] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [loading, setLoading] = useState(true);

  const query = useMemo(() => search.trim(), [search]);

  function load() {
    setLoading(true);
    const path = query
      ? `/api/v1/admin/users?q=${encodeURIComponent(query)}`
      : "/api/v1/admin/users";
    client
      .request<UserRow[]>(path)
      .then(setRows)
      .catch((err) => setError(err.message))
      .finally(() => setLoading(false));
  }

  useEffect(load, [client, query]);

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
        <select name="role" defaultValue="user">
          <option value="user">user</option>
          <option value="admin">admin</option>
        </select>
        <button disabled={submitting}>{submitting ? "Sending…" : "Create invitation"}</button>
      </form>
      <div className="panel filters" style={{ marginTop: 12 }}>
        <input
          type="search"
          placeholder="Search by name, email, or user ID"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label="Search users"
        />
      </div>
      {notice && <p className="panel notice success">{notice}</p>}
      {invite && (
        <p className="panel notice">
          <strong>Invitation link</strong> (shown once):<br />
          <code style={{ wordBreak: "break-all" }}>{invite}</code>
        </p>
      )}
      {error && <p className="panel notice error">{error}</p>}
      <table>
        <thead>
          <tr>
            <th>ID</th>
            <th>Name</th>
            <th>Email</th>
            <th>Role</th>
            <th>Status</th>
            <th>Registered</th>
            <th aria-label="Actions" />
          </tr>
        </thead>
        <tbody>
          {loading && (
            <tr>
              <td colSpan={7}>Loading…</td>
            </tr>
          )}
          {!loading && rows.length === 0 && (
            <tr>
              <td colSpan={7}>No users match your search.</td>
            </tr>
          )}
          {!loading &&
            rows.map((user) => (
              <tr key={user.id}>
                <td>
                  <code style={{ fontSize: "0.85em" }} title={user.id}>
                    {user.id.slice(0, 8)}…
                  </code>
                </td>
                <td>
                  <Link href={`/users/${user.id}`}>{user.display_name}</Link>
                </td>
                <td>{user.email}</td>
                <td>{user.role}</td>
                <td>{user.status}</td>
                <td>{formatWhen(user.created_at)}</td>
                <td>
                  <Link href={`/users/${user.id}`} className="ghost" style={{ fontSize: "0.9em" }}>
                    Edit
                  </Link>
                </td>
              </tr>
            ))}
        </tbody>
      </table>
    </section>
  );
}