"use client";

import { useRouter } from "next/navigation";
import { FormEvent, useState } from "react";
import { Desk } from "@/components/desk";
import { PolymarketAccountCredentialsFields } from "@/components/polymarket-account-credentials-fields";
import type { SignatureTypeValue } from "@/lib/polymarket-account-hints";
import { useSession } from "@/lib/session";

export default function NewAccountPage() {
  return (
    <Desk>
      <Form />
    </Desk>
  );
}

function Form() {
  const { client } = useSession();
  const router = useRouter();
  const [error, setError] = useState("");
  const [signatureType, setSignatureType] = useState<SignatureTypeValue>("eoa");
  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = Object.fromEntries(new FormData(event.currentTarget).entries()) as Record<
      string,
      FormDataEntryValue
    >;
    const pk = String(data.private_key ?? "").trim();
    if (!pk) {
      delete data.private_key;
    } else {
      data.private_key = pk;
    }
    try {
      const created = await client.request<{ id: string }>("/api/v1/accounts", {
        method: "POST",
        body: JSON.stringify(data),
      });
      router.push(`/accounts/${created.id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not add account");
    }
  }
  return (
    <form className="panel account-credentials-form" onSubmit={onSubmit}>
      <h1>Add account</h1>
      <p className="muted">
        Signer and funder addresses are required. Private key is optional — encrypted when provided and needed only for
        Live trading and exchange balance checks. Paper trading works without it.
      </p>
      <PolymarketAccountCredentialsFields
        showName
        signatureType={signatureType}
        onSignatureTypeChange={setSignatureType}
        privateKeyPlaceholder="Private key 0x… (optional)"
      />
      {error && <div className="error">{error}</div>}
      <button type="submit">Save account</button>
    </form>
  );
}
