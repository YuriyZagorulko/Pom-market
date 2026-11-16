"use client";

import Link from "next/link";
import { useParams } from "next/navigation";
import { FormEvent, useCallback, useEffect, useState } from "react";
import { ConfirmationModal } from "@/components/confirmation-modal";
import { Desk } from "@/components/desk";
import { inviteFeedback, type InviteResult } from "@/lib/invite-feedback";
import { useSession } from "@/lib/session";

type AdminUser = {
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

export default function UserPage() {
  const params = useParams<{ id: string }>();
  return (
    <Desk>
      <Detail id={params.id} />
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

function Detail({ id }: { id: string }) {
  const { client } = useSession();
  const [user, setUser] = useState<AdminUser | null>(null);
  const [loadError, setLoadError] = useState("");
  const [profileBusy, setProfileBusy] = useState(false);
  const [profileError, setProfileError] = useState("");
  const [profileNotice, setProfileNotice] = useState("");
  const [passwordBusy, setPasswordBusy] = useState(false);
  const [passwordError, setPasswordError] = useState("");
  const [passwordNotice, setPasswordNotice] = useState("");
  const [passwordOpen, setPasswordOpen] = useState(false);
  const [pendingPassword, setPendingPassword] = useState("");
  const [invite, setInvite] = useState("");
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const [inviting, setInviting] = useState(false);
  const [suspendOpen, setSuspendOpen] = useState(false);
  const [suspendBusy, setSuspendBusy] = useState(false);
  const [suspendError, setSuspendError] = useState("");

  const reload = useCallback(() => {
    setLoadError("");
    client
      .request<AdminUser>(`/api/v1/admin/users/${id}`)
      .then(setUser)
      .catch((err) => setLoadError(err instanceof Error ? err.message : "Could not load user"));
  }, [client, id]);

  useEffect(() => {
    reload();
  }, [reload]);

  async function saveProfile(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setProfileError("");
    setProfileNotice("");
    const body = Object.fromEntries(new FormData(event.currentTarget).entries());
    setProfileBusy(true);
    try {
      await client.request(`/api/v1/admin/users/${id}`, {
        method: "PATCH",
        body: JSON.stringify(body),
      });
      setProfileNotice("Profile saved.");
      reload();
    } catch (err) {
      setProfileError(err instanceof Error ? err.message : "Could not save profile");
    } finally {
      setProfileBusy(false);
    }
  }

  function requestPasswordChange(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setPasswordError("");
    setPasswordNotice("");
    const data = Object.fromEntries(new FormData(event.currentTarget).entries());
    const next = String(data.new_password ?? "");
    const confirm = String(data.confirm_password ?? "");
    if (next.length < 10) {
      setPasswordError("Password must be at least 10 characters.");
      return;
    }
    if (next !== confirm) {
      setPasswordError("Passwords do not match.");
      return;
    }
    setPendingPassword(next);
    setPasswordOpen(true);
  }

  async function confirmPasswordChange() {
    if (passwordBusy || !pendingPassword) return;
    setPasswordBusy(true);
    setPasswordError("");
    try {
      await client.request(`/api/v1/admin/users/${id}/password`, {
        method: "POST",
        body: JSON.stringify({ new_password: pendingPassword }),
      });
      setPasswordNotice("Password updated. Existing sessions for this user were revoked.");
      setPasswordOpen(false);
      setPendingPassword("");
      const form = document.getElementById("admin-password-form") as HTMLFormElement | null;
      form?.reset();
    } catch (err) {
      setPasswordError(err instanceof Error ? err.message : "Could not change password");
      setPasswordOpen(false);
    } finally {
      setPasswordBusy(false);
    }
  }

  if (loadError) {
    return (
      <section>
        <p className="panel notice error">{loadError}</p>
        <Link href="/users">Back to users</Link>
      </section>
    );
  }

  if (!user) return <p>Loading…</p>;

  return (
    <section style={{ display: "grid", gap: 16, maxWidth: 720 }}>
      <div>
        <Link href="/users" className="ghost" style={{ fontSize: "0.9em" }}>
          ← Users
        </Link>
        <h1 style={{ marginTop: 8 }}>{user.display_name}</h1>
        <code style={{ fontSize: "0.85em", opacity: 0.8 }}>{user.id}</code>
      </div>

      <article className="panel" style={{ display: "grid", gap: 12 }}>
        <h2 style={{ margin: 0, fontSize: "1.1rem" }}>Profile information</h2>
        <p style={{ margin: 0, opacity: 0.85, fontSize: "0.9em" }}>
          Display name and email used for login and account emails. There is no separate username field in this product.
        </p>
        <form className="filters" onSubmit={saveProfile} style={{ flexDirection: "column", alignItems: "stretch" }}>
          <label>
            Name
            <input name="display_name" defaultValue={user.display_name} required />
          </label>
          <label>
            Email
            <input name="email" type="email" defaultValue={user.email} required />
          </label>
          <button type="submit" disabled={profileBusy} style={{ alignSelf: "flex-start" }}>
            {profileBusy ? "Saving…" : "Save changes"}
          </button>
        </form>
        {profileNotice && <p className="notice success" style={{ margin: 0 }}>{profileNotice}</p>}
        {profileError && <p className="notice error" style={{ margin: 0 }}>{profileError}</p>}
      </article>

      <article className="panel" style={{ display: "grid", gap: 12 }}>
        <h2 style={{ margin: 0, fontSize: "1.1rem" }}>Change password</h2>
        <p style={{ margin: 0, opacity: 0.85, fontSize: "0.9em" }}>
          Set a new password directly. The user&apos;s active sessions will be revoked after a successful change.
        </p>
        <form
          id="admin-password-form"
          className="filters"
          onSubmit={requestPasswordChange}
          style={{ flexDirection: "column", alignItems: "stretch" }}
        >
          <label>
            New password
            <input name="new_password" type="password" autoComplete="new-password" minLength={10} required />
          </label>
          <label>
            Confirm new password
            <input name="confirm_password" type="password" autoComplete="new-password" minLength={10} required />
          </label>
          <button type="submit" disabled={passwordBusy} style={{ alignSelf: "flex-start" }}>
            Change password
          </button>
        </form>
        {passwordNotice && <p className="notice success" style={{ margin: 0 }}>{passwordNotice}</p>}
        {passwordError && <p className="notice error" style={{ margin: 0 }}>{passwordError}</p>}
      </article>

      <article className="panel" style={{ display: "grid", gap: 8 }}>
        <h2 style={{ margin: 0, fontSize: "1.1rem" }}>Account information</h2>
        <dl style={{ margin: 0, display: "grid", gridTemplateColumns: "auto 1fr", gap: "6px 16px" }}>
          <dt>Role</dt>
          <dd style={{ margin: 0 }}>{user.role}</dd>
          <dt>Status</dt>
          <dd style={{ margin: 0 }}>{user.status}</dd>
          <dt>Registered</dt>
          <dd style={{ margin: 0 }}>{formatWhen(user.created_at)}</dd>
          <dt>Last updated</dt>
          <dd style={{ margin: 0 }}>{formatWhen(user.updated_at)}</dd>
          <dt>Last login</dt>
          <dd style={{ margin: 0 }}>{user.last_login_at ? formatWhen(user.last_login_at) : "Never"}</dd>
          <dt>Trading accounts</dt>
          <dd style={{ margin: 0 }}>{user.accounts}</dd>
          <dt>Source wallets</dt>
          <dd style={{ margin: 0 }}>{user.traders}</dd>
        </dl>
      </article>

      <div className="filters">
        <button
          type="button"
          onClick={() =>
            client
              .request(`/api/v1/admin/users/${id}/activate`, { method: "POST" })
              .then(() => reload())
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
              const result = await client.request<InviteResult>(`/api/v1/admin/users/${id}/invite`, {
                method: "POST",
              });
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
        open={passwordOpen}
        title="Set new password?"
        description={`This will replace the password for ${user.email} and revoke their active sessions.`}
        confirmLabel="Set password"
        destructive
        busy={passwordBusy}
        error={passwordError}
        onCancel={() => {
          if (!passwordBusy) {
            setPasswordOpen(false);
            setPendingPassword("");
          }
        }}
        onConfirm={confirmPasswordChange}
      />
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
            reload();
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