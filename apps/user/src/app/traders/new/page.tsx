"use client";

import { useRouter } from "next/navigation";
import { FormEvent, useState } from "react";
import { Desk } from "@/components/desk";
import { requestWithTimeout } from "@/lib/request-with-timeout";
import { useSession } from "@/lib/session";

export default function NewTraderPage() {
  return <Desk><Form /></Desk>;
}

function Form() {
  const { client } = useSession();
  const router = useRouter();
  const [error, setError] = useState("");
  const [submitting, setSubmitting] = useState(false);
  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (submitting) return;
    const body = Object.fromEntries(new FormData(event.currentTarget).entries());
    const wallet = String(body.wallet_address ?? "").trim();
    setSubmitting(true);
    setError("");
    try {
      const created = await requestWithTimeout<{ id: string }>(client, "/api/v1/source-traders", {
        method: "POST",
        body: JSON.stringify(body),
      });
      router.push(`/traders/${created.id}`);
    } catch (err) {
      const message = err instanceof Error ? err.message : "Could not add wallet";
      setError(message);
      if (message.includes("timed out")) {
        try {
          const traders = await client.request<Array<{ id: string; wallet_address: string }>>("/api/v1/source-traders");
          const found = traders.find((t) => t.wallet_address.toLowerCase() === wallet.toLowerCase());
          if (found) router.push(`/traders/${found.id}`);
        } catch {
          // keep timeout message
        }
      }
    } finally {
      setSubmitting(false);
    }
  }
  return (
    <form className="panel" onSubmit={onSubmit} style={{ display: "grid", gap: 10, maxWidth: 520 }}>
      <h1>Add source wallet</h1>
      <input name="alias" placeholder="Alias" required disabled={submitting} />
      <input name="wallet_address" placeholder="Public 0x address" required disabled={submitting} />
      {error && <div className="error">{error}</div>}
      <button type="submit" disabled={submitting}>{submitting ? "Adding…" : "Check and add"}</button>
      <p className="muted" style={{ margin: 0 }}>
        Polymarket profile lookup runs once at add time (short timeout). Trade history syncs in the background via the worker.
      </p>
    </form>
  );
}
