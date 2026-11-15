"use client";

import { FormEvent, useCallback, useEffect, useState } from "react";
import type { ApiClient } from "@pom/api-client";
import { ConfirmationModal } from "@/components/confirmation-modal";
import {
  STRATEGY_HARD_CLOSE_API_CONFIRMATION,
  StrategyHardCloseConfirmBody,
} from "@/components/strategy-hard-close-copy";
import { useToast } from "@/components/toast";

type ControlState = {
  trading_control_state: string;
  account_trading_control_state?: string;
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

export function StrategyDangerZone({
  strategyId,
  client,
  onChange,
}: {
  strategyId: string;
  client: ApiClient;
  onChange?: () => void;
}) {
  const toast = useToast();
  const [control, setControl] = useState<ControlState | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [preview, setPreview] = useState<Record<string, unknown> | null>(null);
  const [hardCloseModalOpen, setHardCloseModalOpen] = useState(false);
  const [hardCloseModalError, setHardCloseModalError] = useState("");
  const [lastOpId, setLastOpId] = useState<string | null>(null);
  const [opStatus, setOpStatus] = useState<HardCloseOp | null>(null);
  const base = `/api/v1/strategies/${strategyId}/trading-control`;

  const load = useCallback(async () => {
    setError("");
    const data = await client.request<ControlState>(base);
    setControl(data);
  }, [base, client]);

  useEffect(() => {
    void load().catch((e) => setError(e instanceof Error ? e.message : "load failed"));
  }, [load]);

  useEffect(() => {
    if (!lastOpId) return;
    const poll = () => {
      void client
        .request<HardCloseOp>(`${base}/hard-close/${lastOpId}`)
        .then((op) => {
          setOpStatus(op);
          if (op.status !== "queued" && op.status !== "in_progress") {
            void load().catch(() => undefined);
            onChange?.();
          }
        })
        .catch(() => undefined);
    };
    poll();
    const timer = setInterval(poll, 2000);
    return () => clearInterval(timer);
  }, [base, client, lastOpId, load, onChange]);

  async function post(path: string, body: unknown, successMessage: string) {
    setBusy(true);
    setError("");
    try {
      await client.request(path, { method: "POST", body: JSON.stringify(body) });
      await load();
      onChange?.();
      toast.success(successMessage);
    } catch (e) {
      const msg = e instanceof Error ? e.message : "request failed";
      setError(msg);
      toast.error(msg);
    } finally {
      setBusy(false);
    }
  }

  async function onKillActivate(e: FormEvent) {
    e.preventDefault();
    if (
      !confirm(
        "Stop all copy trading for this strategy only? Outstanding BUY orders for this strategy will be cancelled.",
      )
    ) {
      return;
    }
    await post(`${base}/kill-switch/activate`, { confirm: true }, "Strategy kill switch activated");
  }

  async function onKillDeactivate(e: FormEvent) {
    e.preventDefault();
    if (!confirm("Resume normal copy trading for this strategy?")) return;
    await post(`${base}/kill-switch/deactivate`, { confirm: true }, "Strategy kill switch deactivated");
  }

  async function onSoftActivate(e: FormEvent) {
    e.preventDefault();
    if (!confirm("Block new BUYs for this strategy while allowing SELL copies?")) return;
    await post(`${base}/soft-close/activate`, { confirm: true }, "Strategy soft close activated");
  }

  async function onSoftDeactivate(e: FormEvent) {
    e.preventDefault();
    if (!confirm("Return this strategy to normal trading?")) return;
    await post(`${base}/soft-close/deactivate`, { confirm: true }, "Strategy soft close deactivated");
  }

  async function loadPreview() {
    setPreview(await client.request<Record<string, unknown>>(`${base}/hard-close/preview`));
  }

  function openHardCloseModal() {
    if (busy) return;
    setHardCloseModalError("");
    setHardCloseModalOpen(true);
  }

  function closeHardCloseModal() {
    if (busy) return;
    setHardCloseModalOpen(false);
    setHardCloseModalError("");
  }

  async function confirmHardClose() {
    if (busy) return;
    setBusy(true);
    setHardCloseModalError("");
    setError("");
    try {
      const key = `shc-${Date.now()}`;
      const res = await client.request<{ operation_id: string }>(`${base}/hard-close`, {
        method: "POST",
        body: JSON.stringify({ confirmation: STRATEGY_HARD_CLOSE_API_CONFIRMATION }),
        headers: { "Idempotency-Key": key },
      });
      setLastOpId(res.operation_id);
      setHardCloseModalOpen(false);
      await load();
      onChange?.();
      toast.info("Strategy hard close started");
    } catch (e) {
      const msg = e instanceof Error ? e.message : "hard close failed";
      setHardCloseModalError(msg);
      setError(msg);
      toast.error(msg);
    } finally {
      setBusy(false);
    }
  }

  async function onResume(e: FormEvent) {
    e.preventDefault();
    if (!confirm("Resume copy trading for this strategy after a restricted state?")) return;
    await post(`${base}/resume`, { confirm: true }, "Strategy trading resumed");
  }

  async function onRetry() {
    if (!lastOpId) return;
    await post(`${base}/hard-close/${lastOpId}/retry`, {}, "Hard close retry requested");
  }

  if (!control) return <p className="muted">Loading trading control…</p>;

  const accountBlocks =
    control.account_trading_control_state &&
    control.account_trading_control_state !== "normal";

  return (
    <section className="panel" style={{ borderColor: "#c45c3e", display: "grid", gap: 12 }}>
      <h2 style={{ margin: 0, color: "#9a4318" }}>Strategy trading control</h2>
      <p className="muted" style={{ margin: 0 }}>
        Strategy state: <strong>{labelState(control.trading_control_state)}</strong>
        {control.account_trading_control_state && (
          <>
            {" "}
            · Account: <strong>{labelState(control.account_trading_control_state)}</strong>
          </>
        )}
        {control.resume_requires_confirmation && " · resume requires confirmation"}
      </p>
      {accountBlocks && (
        <p className="warn" style={{ margin: 0 }}>
          Account-level restrictions apply and may override this strategy until the account returns to
          normal.
        </p>
      )}
      {error && <p className="error">{error}</p>}

      <div style={{ display: "grid", gap: 10 }}>
        <div>
          <h3 style={{ margin: "0 0 4px" }}>Kill switch</h3>
          <p className="muted" style={{ margin: "0 0 8px" }}>
            Stop BUY and SELL copy for this strategy only. Does not close open positions.
          </p>
          {control.trading_control_state === "kill_switch" ? (
            <form onSubmit={onKillDeactivate}>
              <button type="submit" disabled={busy}>
                Deactivate kill switch
              </button>
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
            Block new BUYs for this strategy; allow copy SELLs to unwind.
          </p>
          {control.trading_control_state === "soft_close" ? (
            <form onSubmit={onSoftDeactivate}>
              <button type="submit" disabled={busy}>
                Disable soft close
              </button>
            </form>
          ) : (
            <form onSubmit={onSoftActivate}>
              <button
                type="submit"
                disabled={
                  busy ||
                  !["normal", "hard_close_completed", "hard_close_partial"].includes(
                    control.trading_control_state,
                  )
                }
              >
                Activate soft close
              </button>
            </form>
          )}
        </div>

        <div>
          <h3 style={{ margin: "0 0 4px" }}>Hard close</h3>
          <p className="muted" style={{ margin: "0 0 8px" }}>
            Paper only: cancel this strategy&apos;s BUY orders and sell all open positions belonging to this
            strategy (not manual lots or other strategies).
          </p>
          <button type="button" className="ghost" disabled={busy} onClick={() => void loadPreview()}>
            Load close preview
          </button>
          {preview && (
            <pre style={{ fontSize: 12, maxHeight: 160, overflow: "auto" }}>
              {JSON.stringify(preview, null, 2)}
            </pre>
          )}
          <button
            type="button"
            className="danger"
            style={{ marginTop: 8 }}
            disabled={busy}
            data-testid="strategy-hard-close-open"
            onClick={openHardCloseModal}
          >
            Hard close strategy positions
          </button>
          <ConfirmationModal
            open={hardCloseModalOpen}
            title="Confirm hard close"
            description={<StrategyHardCloseConfirmBody />}
            confirmLabel="Confirm Hard Close"
            cancelLabel="Cancel"
            destructive
            busy={busy}
            error={hardCloseModalError}
            onConfirm={() => void confirmHardClose()}
            onCancel={closeHardCloseModal}
          />
          {opStatus && (
            <div style={{ marginTop: 8, fontSize: 14 }}>
              <p>
                Operation {opStatus.status}
                {(opStatus.status === "queued" || opStatus.status === "in_progress") &&
                  opStatus.items && (
                    <span className="muted">
                      {" "}
                      ·{" "}
                      {
                        opStatus.items.filter((it) => it.status === "closed" || it.status === "skipped")
                          .length
                      }
                      /{opStatus.items.length} positions processed
                    </span>
                  )}
              </p>
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
            </div>
          )}
        </div>

        {control.resume_requires_confirmation && control.trading_control_state !== "kill_switch" && (
          <form onSubmit={onResume}>
            <button type="submit" disabled={busy}>
              Return strategy to normal
            </button>
          </form>
        )}
      </div>
    </section>
  );
}
