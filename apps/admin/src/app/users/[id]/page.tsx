"use client";

import { useParams } from "next/navigation";
import { FormEvent, useEffect, useState } from "react";
import { Desk } from "@/components/desk";
import { useSession } from "@/lib/session";

export default function UserPage() {
  const params = useParams<{ id: string }>();
  return <Desk><Detail id={params.id} /></Desk>;
}

function Detail({ id }: { id: string }) {
  const { client } = useSession();
  const [user, setUser] = useState<Record<string, string | number | null> | null>(null);
  const [invite, setInvite] = useState("");
  useEffect(() => { client.request<Record<string, string | number | null>>(`/api/v1/admin/users/${id}`).then(setUser); }, [client, id]);
  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const body = Object.fromEntries(new FormData(event.currentTarget).entries());
    await client.request(`/api/v1/admin/users/${id}`, { method: "PATCH", body: JSON.stringify(body) });
    location.reload();
  }
  if (!user) return <p>Loading…</p>;
  return (
    <section style={{ display: "grid", gap: 12 }}>
      <h1>{user.display_name}</h1>
      <div className="panel">
        <div>{user.email}</div>
        <div>Created {user.created_at}</div>
        <div>Last login {user.last_login_at ?? "never"}</div>
        <div>Status {user.status} · accounts {user.accounts} · wallets {user.traders}</div>
      </div>
      <form className="panel filters" onSubmit={save}>
        <input name="display_name" defaultValue={String(user.display_name)} />
        <select name="role" defaultValue={String(user.role)}><option value="user">user</option><option value="admin">admin</option></select>
        <button>Save</button>
      </form>
      <div className="filters">
        <button onClick={() => client.request(`/api/v1/admin/users/${id}/activate`, { method: "POST" }).then(() => location.reload())}>Activate</button>
        <button className="ghost" onClick={() => client.request(`/api/v1/admin/users/${id}/suspend`, { method: "POST" }).then(() => location.reload())}>Suspend</button>
        <button className="ghost" onClick={() => client.request<{ setup_url: string }>(`/api/v1/admin/users/${id}/invite`, { method: "POST" }).then((result) => setInvite(result.setup_url))}>New invitation</button>
      </div>
      {invite && <p>Invitation, shown once: {invite}</p>}
    </section>
  );
}
