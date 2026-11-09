"use client";

import { FormEvent, Suspense, useMemo, useState } from "react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { ApiClient } from "@pom/api-client";

function PasswordInput({
  name,
  label,
  value,
  onChange,
}: {
  name: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  const [visible, setVisible] = useState(false);
  return (
    <div className="password-field">
      <label className="muted" htmlFor={name}>
        {label}
      </label>
      <div className="password-row">
        <input
          id={name}
          name={name}
          type={visible ? "text" : "password"}
          minLength={10}
          required
          value={value}
          onChange={(event) => onChange(event.target.value)}
          autoComplete="new-password"
        />
        <button type="button" className="ghost" onClick={() => setVisible((v) => !v)} aria-pressed={visible}>
          {visible ? "Hide" : "Show"}
        </button>
      </div>
    </div>
  );
}

function ResetPasswordForm() {
  const params = useSearchParams();
  const token = useMemo(() => params.get("token")?.trim() ?? "", [params]);
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState("");
  const [done, setDone] = useState(false);
  const [loading, setLoading] = useState(false);

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!token) {
      setError("This reset link is missing a token.");
      return;
    }
    if (password.length < 10) {
      setError("Password must be at least 10 characters.");
      return;
    }
    if (password !== confirm) {
      setError("Passwords do not match.");
      return;
    }
    setLoading(true);
    setError("");
    try {
      await new ApiClient().request("/api/auth/reset-password", {
        method: "POST",
        body: JSON.stringify({ token, new_password: password }),
      });
      setDone(true);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not reset password");
    } finally {
      setLoading(false);
    }
  }

  if (!token) {
    return (
      <div className="login">
        <div className="panel">
          <h1>Reset password</h1>
          <p className="error">This reset link is invalid or incomplete.</p>
          <Link href="/forgot-password">Request a new link</Link>
        </div>
      </div>
    );
  }

  if (done) {
    return (
      <div className="login">
        <div className="panel">
          <h1>Password updated</h1>
          <p className="ok">Your password was reset. You can sign in with your new password.</p>
          <Link href="/login" className="button">
            Go to login
          </Link>
        </div>
      </div>
    );
  }

  return (
    <div className="login">
      <form className="panel" onSubmit={onSubmit}>
        <h1>Reset password</h1>
        <p className="muted">Choose a new password with at least 10 characters.</p>
        <PasswordInput name="password" label="New password" value={password} onChange={setPassword} />
        <PasswordInput name="confirm" label="Confirm password" value={confirm} onChange={setConfirm} />
        {error && <div className="error">{error}</div>}
        <button type="submit" disabled={loading}>
          {loading ? "Saving…" : "Save new password"}
        </button>
        <Link href="/login" className="muted" style={{ textAlign: "center" }}>
          Back to login
        </Link>
      </form>
    </div>
  );
}

export default function ResetPasswordPage() {
  return (
    <Suspense>
      <ResetPasswordForm />
    </Suspense>
  );
}
