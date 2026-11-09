"use client";

import { useRouter } from "next/navigation";
import { FormEvent, useState } from "react";
import { Desk } from "@/components/desk";
import { useSession } from "@/lib/session";

export default function NewAccountPage() {
  return <Desk><Form /></Desk>;
}

function Form() {
  const { client } = useSession();
  const router = useRouter();
  const [error, setError] = useState("");
  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = Object.fromEntries(new FormData(event.currentTarget).entries());
    try {
      const created = await client.request<{ id: string }>("/api/v1/accounts", { method: "POST", body: JSON.stringify(data) });
      router.push(`/accounts/${created.id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not add account");
    }
  }
  return (
    <form className="panel" onSubmit={onSubmit} style={{ display: "grid", gap: 10, maxWidth: 560 }}>
      <h1>Add account</h1>
      <p className="muted">The private key is encrypted before it is stored. A seed phrase is not accepted. Leave the key empty for monitoring only.</p>
      <input name="name" placeholder="Name" required />
      <select name="signature_type" defaultValue="eoa">
        <option value="eoa">EOA</option>
        <option value="poly_proxy">Proxy / email wallet</option>
        <option value="poly_gnosis_safe">Gnosis Safe</option>
        <option value="poly_1271">EIP-1271 deposit wallet</option>
      </select>
      <input name="signer_address" placeholder="Signer address 0x" required />
      <input name="funder_address" placeholder="Funder / proxy address, if different" />
      <input name="private_key" placeholder="Private key 0x… optional" autoComplete="off" />
      {error && <div className="error">{error}</div>}
      <button type="submit">Save account</button>
    </form>
  );
}
