"use client";

import { useRouter } from "next/navigation";
import { FormEvent, useState } from "react";
import { Desk } from "@/components/desk";
import { PolymarketAccountCredentialsFields } from "@/components/polymarket-account-credentials-fields";
import { SimpleFieldLabel } from "@/components/simple-field-label";
import type { AccountTypeValue } from "@/lib/account-forms";
import { buildCreateAccountBody } from "@/lib/account-forms";
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
  const [accountType, setAccountType] = useState<AccountTypeValue>("paper");
  const [signatureType, setSignatureType] = useState<SignatureTypeValue>("eoa");

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = Object.fromEntries(new FormData(event.currentTarget).entries()) as Record<
      string,
      FormDataEntryValue
    >;
    const built = buildCreateAccountBody(data);
    if (!built.ok) {
      setError(built.error);
      return;
    }
    setError("");
    try {
      const created = await client.request<{ id: string }>("/api/v1/accounts", {
        method: "POST",
        body: JSON.stringify(built.body),
      });
      router.push(`/accounts/${created.id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not add account");
    }
  }

  return (
    <form className="panel account-credentials-form" onSubmit={onSubmit}>
      <h1>Add account</h1>
      <div className="form-field">
        <SimpleFieldLabel htmlFor="account-type">Account type</SimpleFieldLabel>
        <select
          id="account-type"
          name="account_type"
          value={accountType}
          onChange={(e) => setAccountType(e.target.value as AccountTypeValue)}
        >
          <option value="paper">Paper Trading Account</option>
          <option value="wallet">Polymarket Wallet Account</option>
        </select>
      </div>
      {accountType === "paper" ? (
        <>
          <p className="muted">
            Paper trading only — no Polymarket wallet, signer, or private key. Use this account for paper copy
            strategies with virtual capital.
          </p>
          <PolymarketAccountCredentialsFields showName signatureType="eoa" onSignatureTypeChange={() => {}} hideWalletFields />
        </>
      ) : (
        <>
          <p className="muted">
            Signer and funder addresses are required. Private key is optional — encrypted when provided and needed only
            for Live trading and exchange balance checks. Paper trading works without it.
          </p>
          <PolymarketAccountCredentialsFields
            showName
            signatureType={signatureType}
            onSignatureTypeChange={setSignatureType}
            privateKeyPlaceholder="Private key 0x… (optional)"
          />
        </>
      )}
      {error && <div className="error">{error}</div>}
      <button type="submit">Save account</button>
    </form>
  );
}