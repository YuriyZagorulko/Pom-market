"use client";

import { FormEvent, useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useSession } from "@/lib/session";

export default function LoginPage() {
  const { login } = useSession();
  const router = useRouter();
  const [error, setError] = useState("");
  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    try {
      await login(String(data.get("email")), String(data.get("password")));
      router.push("/");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Login failed");
    }
  }
  return (
    <div className="login">
      <form className="panel" onSubmit={onSubmit}>
        <h1>User dashboard</h1>
        <p className="muted">Sign in with the password from your invitation. There is no public registration.</p>
        <input name="email" type="email" placeholder="Email" required />
        <input name="password" type="password" placeholder="Password" required />
        <Link href="/forgot-password" className="muted" style={{ fontSize: 14 }}>
          Forgot password?
        </Link>
        {error && <div className="error">{error}</div>}
        <button type="submit">Enter</button>
      </form>
    </div>
  );
}
