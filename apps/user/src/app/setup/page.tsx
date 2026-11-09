"use client";

import { FormEvent, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense } from "react";
import { ApiClient } from "@pom/api-client";

function SetupForm() {
  const params = useSearchParams();
  const router = useRouter();
  const [error, setError] = useState("");
  const [done, setDone] = useState(false);
  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const password = String(new FormData(event.currentTarget).get("password"));
    try {
      await new ApiClient().request("/api/v1/auth/password/setup", {
        method: "POST",
        body: JSON.stringify({ token: params.get("token"), password }),
      });
      setDone(true);
      setTimeout(() => router.push("/login"), 600);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not set password");
    }
  }
  return (
    <div className="login">
      <form className="panel" onSubmit={onSubmit}>
        <h1>Set your password</h1>
        <p className="muted">This invitation is single-use. Choose at least 10 characters.</p>
        <input name="password" type="password" minLength={10} required />
        {error && <div className="error">{error}</div>}
        {done && <div className="ok">Password saved.</div>}
        <button type="submit">Save password</button>
      </form>
    </div>
  );
}

export default function SetupPage() {
  return <Suspense><SetupForm /></Suspense>;
}
