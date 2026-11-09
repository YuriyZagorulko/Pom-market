"use client";

import { FormEvent, useState } from "react";
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
        <h1>Admin console</h1>
        <p className="muted">Operators sign in here. Trading keys are not available in this console.</p>
        <input name="email" type="email" required placeholder="Admin email" />
        <input name="password" type="password" required placeholder="Password" />
        {error && <div className="error">{error}</div>}
        <button>Sign in</button>
      </form>
    </div>
  );
}
