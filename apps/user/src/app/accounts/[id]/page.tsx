"use client";

import { useParams } from "next/navigation";
import { FormEvent, useEffect, useState } from "react";
import { DangerZone } from "@/components/danger-zone";
import { Desk, useResource } from "@/components/desk";
import { PolymarketAccountCredentialsFields } from "@/components/polymarket-account-credentials-fields";
import { SimpleFieldLabel } from "@/components/simple-field-label";
import { useToast } from "@/components/toast";
import {
  accountRiskFormValues,
  buildAccountModeBody,
  buildAccountRenameBody,
  buildAccountRiskPatchBody,
  buildCredentialsRotateBody,
} from "@/lib/account-forms";
import type { SignatureTypeValue } from "@/lib/polymarket-account-hints";
import { useSession } from "@/lib/session";

export default function AccountPage() {
  const params = useParams<{ id: string }>();
  return (
    <Desk>
      <Detail id={params.id} />
    </Desk>
  );
}

function Detail({ id }: { id: string }) {
  const { client } = useSession();
  const toast = useToast();
  const { data, error, reload } = useResource<{
    account: Record<string, string | null>;
    risk: Record<string, string | number>;
    credentials_stored: boolean;
  }>(`/api/v1/accounts/${id}`);
  const [report, setReport] = useState<Record<string, unknown> | null>(null);
  const [credentialError, setCredentialError] = useState("");
  const [signatureType, setSignatureType] = useState<SignatureTypeValue>("eoa");

  async function test() {
    try {
      setReport(await client.request(`/api/v1/accounts/${id}/test`, { method: "POST" }));
      toast.success("Connection test finished — see result below.");
      reload();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Connection test failed");
    }
  }

  async function mode(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setCredentialError("");
    const form = Object.fromEntries(new FormData(event.currentTarget).entries());
    const built = buildAccountModeBody(form);
    if (!built.ok) {
      toast.error(built.error);
      return;
    }
    try {
      await client.request(`/api/v1/accounts/${id}/mode`, { method: "POST", body: JSON.stringify(built.body) });
      toast.success("Operating mode updated");
      reload();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Could not update mode");
    }
  }

  async function risk(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = Object.fromEntries(new FormData(event.currentTarget).entries());
    const built = buildAccountRiskPatchBody(form);
    if (!built.ok) {
      toast.error(built.error);
      return;
    }
    try {
      await client.request(`/api/v1/accounts/${id}/risk`, { method: "PATCH", body: JSON.stringify(built.body) });
      toast.success("Account risk limits saved");
      reload();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Could not save risk limits");
    }
  }

  async function rename(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = Object.fromEntries(new FormData(event.currentTarget).entries());
    const built = buildAccountRenameBody(form);
    if (!built.ok) {
      toast.error(built.error);
      return;
    }
    try {
      await client.request(`/api/v1/accounts/${id}`, { method: "PATCH", body: JSON.stringify(built.body) });
      toast.success("Account renamed");
      reload();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Could not rename account");
    }
  }

  async function disconnect() {
    try {
      await client.request(`/api/v1/accounts/${id}/disconnect`, { method: "POST" });
      toast.info("Account disconnected");
      reload();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Disconnect failed");
    }
  }

  async function remove() {
    if (!confirm("Delete this account and its strategies? History attached to it is removed with it.")) return;
    try {
      await client.request(`/api/v1/accounts/${id}`, { method: "DELETE" });
      toast.info("Account deleted");
      location.href = "/accounts";
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Delete failed");
    }
  }

  useEffect(() => {
    const sig = data?.account.signature_type;
    if (sig === "eoa" || sig === "poly_proxy" || sig === "poly_gnosis_safe" || sig === "poly_1271") {
      setSignatureType(sig);
    }
  }, [data?.account.signature_type]);

  async function rotateCredentials(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setCredentialError("");
    const form = Object.fromEntries(new FormData(event.currentTarget).entries());
    const body = buildCredentialsRotateBody(form, data?.account.name ?? "");
    try {
      await client.request(`/api/v1/accounts/${id}/credentials/rotate`, {
        method: "POST",
        body: JSON.stringify(body),
      });
      toast.success("Polymarket credentials updated");
      reload();
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Could not update credentials";
      setCredentialError(msg);
      toast.error(msg);
    }
  }

  if (error) return <p className="error">{error}</p>;
  if (!data) return <p>Loading…</p>;
  const account = data.account;
  const riskDefaults = accountRiskFormValues(data.risk);

  return (
    <section style={{ display: "grid", gap: 16 }}>
      <h1>{account.name}</h1>
      <div className="panel">
        <div>Signer {account.signer_address}</div>
        <div>Funder {account.funder_address}</div>
        <div>
          Signature {account.signature_type} · Mode {account.operating_mode} · {account.status}
        </div>
        <div>
          Exchange balance {account.last_exchange_balance_usd ?? "not checked"}
          {account.last_connection_error && <span className="error"> · {account.last_connection_error}</span>}
        </div>
        <div>Credentials stored: {data.credentials_stored ? "yes" : "no"}. The key itself is never shown.</div>
        <div className="filters">
          <button type="button" onClick={test}>
            Test connection
          </button>
          <button type="button" className="ghost" onClick={disconnect}>
            Disconnect
          </button>
          <button type="button" className="ghost" onClick={remove}>
            Delete
          </button>
        </div>
        {report && <pre>{JSON.stringify(report, null, 2)}</pre>}
      </div>

      <form className="panel form-stack" onSubmit={rename}>
        <h2 style={{ margin: 0 }}>Account name</h2>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="account-rename">Display name</SimpleFieldLabel>
          <input id="account-rename" name="name" defaultValue={String(account.name)} />
        </div>
        <button type="submit">Save name</button>
      </form>

      <form className="panel account-credentials-form" onSubmit={rotateCredentials}>
        <h2>Polymarket credentials</h2>
        <p className="muted">Update signer, funder, signature type, or private key. The stored key is never shown.</p>
        <PolymarketAccountCredentialsFields
          key={String(account.signer_address) + String(account.funder_address)}
          signatureType={signatureType}
          onSignatureTypeChange={setSignatureType}
          signerDefault={String(account.signer_address ?? "")}
          funderDefault={String(account.funder_address ?? "")}
          privateKeyRequired
          privateKeyPlaceholder="New private key 0x…"
        />
        {credentialError && <div className="error">{credentialError}</div>}
        <button type="submit">Update credentials</button>
      </form>

      <form className="panel form-stack" onSubmit={mode}>
        <h2 style={{ margin: 0 }}>Operating mode</h2>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="operating-mode">Mode</SimpleFieldLabel>
          <select id="operating-mode" name="mode" defaultValue={String(account.operating_mode)}>
            <option value="monitoring">Monitoring</option>
            <option value="paper_trading">Paper trading</option>
            <option value="live_trading">Live trading</option>
          </select>
        </div>
        <label>
          <input name="confirm_live" type="checkbox" /> I understand live orders spend real funds
        </label>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="live-confirmation" optional>
            Live confirmation phrase
          </SimpleFieldLabel>
          <input
            id="live-confirmation"
            name="confirmation"
            placeholder="Type ENABLE LIVE TRADING for live mode"
          />
        </div>
        <button type="submit">Update mode</button>
      </form>

      <form className="panel form-stack" onSubmit={risk}>
        <h2 style={{ margin: 0 }}>Account risk</h2>
        <p className="muted" style={{ margin: 0 }}>
          Limits and paper cash for this trading account.
        </p>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="max_order_usd">Max order (USD)</SimpleFieldLabel>
          <input id="max_order_usd" name="max_order_usd" defaultValue={riskDefaults.max_order_usd} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="max_daily_loss_usd">Max daily loss (USD)</SimpleFieldLabel>
          <input id="max_daily_loss_usd" name="max_daily_loss_usd" defaultValue={riskDefaults.max_daily_loss_usd} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="max_exposure_usd">Max exposure (USD)</SimpleFieldLabel>
          <input id="max_exposure_usd" name="max_exposure_usd" defaultValue={riskDefaults.max_exposure_usd} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="max_slippage_bps">Max slippage (basis points)</SimpleFieldLabel>
          <input
            id="max_slippage_bps"
            name="max_slippage_bps"
            defaultValue={riskDefaults.max_slippage_bps}
            inputMode="numeric"
          />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="paper_cash_usd">Paper cash balance (USD)</SimpleFieldLabel>
          <input id="paper_cash_usd" name="paper_cash_usd" defaultValue={riskDefaults.paper_cash_usd} />
        </div>
        <button type="submit">Save risk</button>
      </form>

      <DangerZone accountId={id} client={client} onChange={reload} />
    </section>
  );
}
