"use client";

import { useParams } from "next/navigation";
import { FormEvent, useEffect, useState } from "react";
import { ConfirmationModal } from "@/components/confirmation-modal";
import { Desk } from "@/components/desk";
import { inviteFeedback, type InviteResult } from "@/lib/invite-feedback";
import { useSession } from "@/lib/session";

export default function UserPage() {
  const params = useParams<{ id: string }>();
  return <Desk><Detail id={params.id} /></Desk>;
}

function Detail({ id }: { id: string }) {
  const { client } = useSession();
  const [user, setUser] = useState<Record<string, string | number | null> | null>(null);
  const [invite, setInvite] = useState("");
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const [inviting, setInviting] = useState(false);
  const [suspendOpen, setSuspendOpen] = useState(false);
  const [suspendBusy, setSuspendBusy] = useState(false);
  const [suspendError, setSuspendError] = useState("");
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
        <button
          type="button"
          onClick={() =>
            client
              .request(`/api/v1/admin/users/${id}/activate`, { method: "POST" })
              .then(() => location.reload())
          }
        >
          Activate
        </button>
        <button
          type="button"
          className="ghost"
          onClick={() => {
            setSuspendError("");
            setSuspendOpen(true);
          }}
        >
          Suspend
        </button>
        <button
          className="ghost"
          disabled={inviting}
          onClick={async () => {
            setNotice("");
            setError("");
            setInvite("");
            setInviting(true);
            try {
              const result = await client.request<InviteResult>(`/api/v1/admin/users/${id}/invite`, { method: "POST" });
              const feedback = inviteFeedback(result);
              if (feedback.ok) setNotice(feedback.message);
              else setError(feedback.message);
              if (result.setup_url) setInvite(result.setup_url);
            } catch (err) {
              setError(err instanceof Error ? err.message : "Could not send invitation");
            } finally {
              setInviting(false);
            }
          }}
        >
          {inviting ? "Sending…" : "New invitation"}
        </button>
      </div>
      {notice && <p className="panel notice success">{notice}</p>}
      {invite && (
        <p className="panel notice">
          <strong>Invitation link</strong> (shown once):<br />
          <code style={{ wordBreak: "break-all" }}>{invite}</code>
        </p>
      )}
      {error && <p className="panel notice error">{error}</p>}
      <ConfirmationModal
        open={suspendOpen}
        title="Suspend user?"
        description={`${user.email} will lose access until an administrator activates the account again.`}
        confirmLabel="Suspend user"
        destructive
        busy={suspendBusy}
        error={suspendError}
        onCancel={() => {
          if (!suspendBusy) setSuspendOpen(false);
        }}
        onConfirm={async () => {
          if (suspendBusy) return;
          setSuspendBusy(true);
          setSuspendError("");
          try {
            await client.request(`/api/v1/admin/users/${id}/suspend`, { method: "POST" });
            setSuspendOpen(false);
            location.reload();
          } catch (err) {
            setSuspendError(err instanceof Error ? err.message : "Suspend failed");
          } finally {
            setSuspendBusy(false);
          }
        }}
      />
    </section>
  );
}
