"use client";

import { FormEvent, useState } from "react";
import Link from "next/link";
import { ApiClient } from "@pom/api-client";

const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

export default function ForgotPasswordPage() {
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [loading, setLoading] = useState(false);

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const email = String(new FormData(event.currentTarget).get("email")).trim();
    if (!EMAIL_PATTERN.test(email)) {
      setError("Enter a valid email address.");
      setMessage("");
      return;
    }
    setLoading(true);
    setError("");
    try {
      const response = await new ApiClient().request<{ message: string }>("/api/auth/forgot-password", {
        method: "POST",
        body: JSON.stringify({ email }),
      });
      setMessage(response.message);
    } catch (err) {
      setMessage("");
      setError(err instanceof Error ? err.message : "Could not send reset link");
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="login">
      <form className="panel" onSubmit={onSubmit}>
        <h1>Forgot password</h1>
        <p className="muted">Enter your email and we&apos;ll send a reset link if an account exists.</p>
        <input name="email" type="email" placeholder="Email" required autoComplete="email" />
        {error && <div className="error">{error}</div>}
        {message && <div className="ok">{message}</div>}
        <button type="submit" disabled={loading}>
          {loading ? "Sending…" : "Send reset link"}
        </button>
        <Link href="/login" className="muted" style={{ textAlign: "center" }}>
          Back to login
        </Link>
      </form>
    </div>
  );
}
