"use client";

import { useCallback, useEffect, useState } from "react";
import { ConfirmationModal } from "@/components/confirmation-modal";
import { Desk } from "@/components/desk";
import { parseKillSwitch, type OverviewResponse } from "@/lib/overview-metrics";
import { useSession } from "@/lib/session";

type PendingKill = "on" | "off";

export default function SettingsPage() {
  return (
    <Desk>
      <Settings />
    </Desk>
  );
}

function Settings() {
  const { client } = useSession();
  const [enabled, setEnabled] = useState(false);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState("");
  const [notice, setNotice] = useState("");
  const [pending, setPending] = useState<PendingKill | null>(null);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState("");

  const refreshKill = useCallback(() => {
    setLoading(true);
    setLoadError("");
    client
      .request<OverviewResponse>("/api/v1/admin/overview")
      .then((data) => {
        const kill = parseKillSwitch(data.kill_switch);
        if (kill === null) {
          setLoadError("Could not read kill switch status from the server.");
          return;
        }
        setEnabled(kill);
      })
      .catch((err) =>
        setLoadError(err instanceof Error ? err.message : "Could not load settings"),
      )
      .finally(() => setLoading(false));
  }, [client]);

  useEffect(() => {
    refreshKill();
  }, [refreshKill]);

  async function confirmKill() {
    if (!pending || busy) return;
    const next = pending === "on";
    setBusy(true);
    setActionError("");
    try {
      await client.request<{ enabled: boolean }>("/api/v1/admin/kill-switch", {
        method: "POST",
        body: JSON.stringify({ enabled: next }),
      });
      setEnabled(next);
      setNotice(next ? "Kill switch is now ON." : "Kill switch is now OFF.");
      setPending(null);
    } catch (err) {
      setActionError(
        err instanceof Error ? err.message : "Kill switch update failed",
      );
    } finally {
      setBusy(false);
    }
  }

  const modalCopy =
    pending === "on"
      ? {
          title: "Turn Kill Switch ON?",
          description:
            "The global kill switch stops new live orders. It does not delete history, positions, or strategies. Paper and monitoring continue.",
          confirmLabel: "Turn Kill Switch ON",
          destructive: true,
        }
      : pending === "off"
        ? {
            title: "Turn Kill Switch OFF?",
            description:
              "Turning off the global kill switch allows live order creation again, subject to account and strategy settings.",
            confirmLabel: "Turn Kill Switch OFF",
            destructive: true,
          }
        : null;

  return (
    <section>
      <h1>Settings</h1>
      <div className="panel">
        <h2 style={{ marginTop: 0, fontSize: 18 }}>Kill Switch</h2>
        {loading ? (
          <p className="muted">Loading status…</p>
        ) : loadError ? (
          <p className="error">{loadError}</p>
        ) : (
          <>
            <p className="row">
              <span>Current state</span>
              <strong className={`kill-badge${enabled ? " kill-on" : " kill-off"}`}>
                {enabled ? "ON" : "OFF"}
              </strong>
            </p>
            <p className="muted">
              The global kill switch stops new live orders. It does not delete history,
              positions, or strategies. Paper and monitoring continue.
            </p>
            <button
              type="button"
              disabled={busy}
              onClick={() => {
                setNotice("");
                setActionError("");
                setPending(enabled ? "off" : "on");
              }}
            >
              {enabled ? "Turn kill switch OFF" : "Turn kill switch ON"}
            </button>
          </>
        )}
        {notice ? <p className="notice success">{notice}</p> : null}
      </div>
      <ConfirmationModal
        open={pending !== null}
        title={modalCopy?.title ?? ""}
        description={modalCopy?.description ?? ""}
        confirmLabel={modalCopy?.confirmLabel ?? "Confirm"}
        destructive={modalCopy?.destructive}
        busy={busy}
        error={actionError}
        onCancel={() => {
          if (!busy) setPending(null);
        }}
        onConfirm={() => void confirmKill()}
      />
    </section>
  );
}
