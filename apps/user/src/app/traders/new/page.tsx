"use client";

import { useRouter } from "next/navigation";
import { FormEvent, useState } from "react";
import { Desk } from "@/components/desk";
import { useSession } from "@/lib/session";

export default function NewTraderPage() {
  return <Desk><Form /></Desk>;
}

function Form() {
  const { client } = useSession();
  const router = useRouter();
  const [error, setError] = useState("");
  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const body = Object.fromEntries(new FormData(event.currentTarget).entries());
    try {
      const created = await client.request<{ id: string }>("/api/v1/source-traders", { method: "POST", body: JSON.stringify(body) });
      router.push(`/traders/${created.id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not add wallet");
    }
  }
  return (
    <form className="panel" onSubmit={onSubmit} style={{ display: "grid", gap: 10, maxWidth: 520 }}>
      <h1>Add source wallet</h1>
      <input name="alias" placeholder="Alias" required />
      <input name="wallet_address" placeholder="Public 0x address" required />
      {error && <div className="error">{error}</div>}
      <button>Check and add</button>
    </form>
  );
}
