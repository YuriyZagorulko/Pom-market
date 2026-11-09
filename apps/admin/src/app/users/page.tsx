"use client";

import Link from "next/link";
import { FormEvent, useEffect, useState } from "react";
import { Desk } from "@/components/desk";
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
  function load() { client.request<UserRow[]>("/api/v1/admin/users").then(setRows).catch((err) => setError(err.message)); }
  useEffect(load, [client]);
  async function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const body = Object.fromEntries(new FormData(event.currentTarget).entries());
    const created = await client.request<{ setup_url: string }>("/api/v1/admin/users", { method: "POST", body: JSON.stringify(body) });
    setInvite(created.setup_url);
    load();
  }
  return (
    <section>
      <h1>Users</h1>
      <form className="panel filters" onSubmit={create}>
        <input name="email" type="email" required placeholder="Email" />
        <input name="display_name" required placeholder="Display name" />
        <select name="role" defaultValue="user"><option value="user">user</option><option value="admin">admin</option></select>
        <button>Create invitation</button>
      </form>
      {invite && <p>Invitation, shown once: {invite}</p>}
      {error && <p className="error">{error}</p>}
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
