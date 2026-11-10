"use client";

import { FormEvent, useCallback, useEffect, useState } from "react";
import type { ApiClient } from "@pom/api-client";

type ControlState = {
  trading_control_state: string;
  blocks_buy: boolean;
  blocks_all_trading: boolean;
  allows_copy_sell: boolean;
  resume_requires_confirmation: boolean;
};

type HardCloseOp = {
  id: string;
  status: string;
  items?: Array<{
    token_id: string;
    status: string;
    remaining_shares: string;
    closed_shares: string;
    error?: string | null;
  }>;
};

function labelState(state: string): string {
  return state.replace(/_/g, " ");
}

export function DangerZone({
  accountId,
  client,
  onChange,
}: {
  accountId: string;
  client: ApiClient;
  onChange?: () => void;
}) {
  const [control, setControl] = useState<ControlState | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [preview, setPreview] = useState<Record<string, unknown> | null>(null);
  const [hardClosePhrase, setHardClosePhrase] = useState("");
  const [lastOpId, setLastOpId] = useState<string | null>(null);
  const [opStatus, setOpStatus] = useState<HardCloseOp | null>(null);

  const load = useCallback(async () => {
    setError("");
    const data = await client.request<ControlState>(
      `/api/v1/accounts/${accountId}/trading-control`,
    );
    setControl(data);
  }, [accountId, client]);

  useEffect(() => {
    void load().catch((e) => setError(e instanceof Error ? e.message : "load failed"));
  }, [load]);

  useEffect(() => {
    if (!lastOpId) return;
    const timer = setInterval(() => {
      void client
        .request<HardCloseOp>(`/api/v1/accounts/${accountId}/hard-close/${lastOpId}`)
        .then(setOpStatus)
        .catch(() => undefined);
    }, 2000);
    return () => clearInterval(timer);
  }, [accountId, client, lastOpId]);

  async function post(path: string, body: unknown) {
    setBusy(true);
    setError("");
    try {
      await client.request(path, { method: "POST", body: JSON.stringify(body) });
      await load();
      onChange?.();
    } catch (e) {
      setError(e instanceof Error ? e.message : "request failed");
    } finally {
      setBusy(false);
    }
  }

  async function onKillActivate(e: FormEvent) {
    e.preventDefault();
    if (!confirm("Stop ALL trading for this account? Outstanding BUY orders will be cancelled.")) return;
    await post(`/api/v1/accounts/${accountId}/trading-control/kill-switch/activate`, { confirm: true });
  }

  async function onKillDeactivate(e: FormEvent) {
    e.preventDefault();
    if (!confirm("Resume normal trading for this account?")) return;
    await post(`/api/v1/accounts/${accountId}/trading-control/kill-switch/deactivate`, { confirm: true });
  }

  async function onSoftActivate(e: FormEvent) {
    e.preventDefault();
    if (!confirm("Block new BUYs while allowing SELLs to close positions?")) return;
    await post(`/api/v1/accounts/${accountId}/trading-control/soft-close/activate`, { confirm: true });
  }

  async function onSoftDeactivate(e: FormEvent) {
    e.preventDefault();
    if (!confirm("Return to normal trading?")) return;
    await post(`/api/v1/accounts/${accountId}/trading-control/soft-close/deactivate`, { confirm: true });
  }

  async function loadPreview() {
    setPreview(
      await client.request<Record<string, unknown>>(
        `/api/v1/accounts/${accountId}/hard-close/preview`,
      ),
    );
  }

  async function onHardClose(e: FormEvent) {
    e.preventDefault();
    if (hardClosePhrase !== "CLOSE ALL") {
      setError("Type CLOSE ALL to confirm");
      return;
    }
    setBusy(true);
    try {
      const key = `hc-${Date.now()}`;
      const res = await client.request<{ operation_id: string }>(
        `/api/v1/accounts/${accountId}/hard-close`,
        {
          method: "POST",
          body: JSON.stringify({ confirmation: "CLOSE ALL" }),
          headers: { "Idempotency-Key": key },
        },
      );
      setLastOpId(res.operation_id);
      await load();
      onChange?.();
    } catch (e) {
      setError(e instanceof Error ? e.message : "hard close failed");
    } finally {
      setBusy(false);
    }
  }

  async function onResume(e: FormEvent) {
    e.preventDefault();
    if (!confirm("Confirm you want to resume trading after a restricted state.")) return;
    await post(`/api/v1/accounts/${accountId}/trading-control/resume`, { confirm: true });
  }

  async function onRetry() {
    if (!lastOpId) return;
    await post(`/api/v1/accounts/${accountId}/hard-close/${lastOpId}/retry`, {});
  }

  if (!control) return <p className="muted">Loading danger zone…</p>;

  return (
    <section className="panel" style={{ borderColor: "#c45c3e", display: "grid", gap: 12 }}>
      <h2 style={{ margin: 0, color: "#9a4318" }}>Danger zone</h2>
      <p className="muted" style={{ margin: 0 }}>
        Current state: <strong>{labelState(control.trading_control_state)}</strong>
        {control.resume_requires_confirmation && " · resume requires confirmation"}
      </p>
      {error && <p className="error">{error}</p>}

      <div style={{ display: "grid", gap: 10 }}>
        <div>
          <h3 style={{ margin: "0 0 4px" }}>Kill switch</h3>
          <p className="muted" style={{ margin: "0 0 8px" }}>
            Stop all trading for this account. Cancels outstanding BUY orders where possible.
          </p>
          {control.trading_control_state === "kill_switch" ? (
            <form onSubmit={onKillDeactivate}>
              <button type="submit" disabled={busy}>Deactivate kill switch</button>
            </form>
          ) : (
            <form onSubmit={onKillActivate}>
              <button type="submit" className="danger" disabled={busy}>
                Activate kill switch
              </button>
            </form>
          )}
        </div>

        <div>
          <h3 style={{ margin: "0 0 4px" }}>Soft close</h3>
          <p className="muted" style={{ margin: "0 0 8px" }}>
            Block new BUYs; allow copy and manual SELLs to reduce positions.
          </p>
          {control.trading_control_state === "soft_close" ? (
            <form onSubmit={onSoftDeactivate}>
              <button type="submit" disabled={busy}>Disable soft close</button>
            </form>
          ) : (
            <form onSubmit={onSoftActivate}>
              <button type="submit" disabled={busy || control.trading_control_state !== "normal"}>
                Activate soft close
              </button>
            </form>
          )}
        </div>

        <div>
          <h3 style={{ margin: "0 0 4px" }}>Hard close</h3>
          <p className="muted" style={{ margin: "0 0 8px" }}>
            Cancel BUY orders and attempt to sell all open positions (paper only).
          </p>
          <button type="button" className="ghost" disabled={busy} onClick={() => void loadPreview()}>
            Load close preview
          </button>
          {preview && (
            <pre style={{ fontSize: 12, maxHeight: 160, overflow: "auto" }}>
              {JSON.stringify(preview, null, 2)}
            </pre>
          )}
          <form onSubmit={onHardClose} style={{ display: "grid", gap: 8, marginTop: 8 }}>
            <input
              value={hardClosePhrase}
              onChange={(e) => setHardClosePhrase(e.target.value)}
              placeholder='Type CLOSE ALL'
              autoComplete="off"
            />
            <button type="submit" className="danger" disabled={busy}>
              Hard close all positions
            </button>
          </form>
          {opStatus && (
            <HardCloseProgress>
              <p>Operation {opStatus.status}</p>
              {opStatus.items?.map((it) => (
                <div key={it.token_id}>
                  {it.token_id}: {it.status} (closed {it.closed_shares}, left {it.remaining_shares})
                  {it.error && <span className="error"> · {it.error}</span>}
                </div>
              ))}
              {(opStatus.status === "partial" || opStatus.status === "failed") && (
                <button type="button" onClick={() => void onRetry()} disabled={busy}>
                  Retry unfinished
                </button>
              )}
            </HardCloseProgress>
          )}
        </div>

        {control.resume_requires_confirmation && control.trading_control_state !== "kill_switch" && (
          <form onSubmit={onResume}>
            <button type="submit" disabled={busy}>
              Resume normal trading
            </button>
          </form>
        )}
      </div>
    </section>
  );
}

function HardCloseProgress({ children }: { children: React.ReactNode }) {
  return <div style={{ marginTop: 8, fontSize: 14 }}>{children}</div>;
}
