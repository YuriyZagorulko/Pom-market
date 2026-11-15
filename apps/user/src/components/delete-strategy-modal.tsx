"use client";

import { useId, useState } from "react";
import { ConfirmationModal } from "@/components/confirmation-modal";

const CONFIRM_PHRASE = "DELETE";

export type DeleteStrategyPreview = {
  can_delete: boolean;
  blockers: string[];
  warnings: string[];
  paper_trading: boolean;
};

export type DeleteStrategyModalProps = {
  open: boolean;
  strategyName: string;
  busy?: boolean;
  previewLoading?: boolean;
  preview?: DeleteStrategyPreview;
  error?: string;
  onConfirm: () => void;
  onCancel: () => void;
};

export function DeleteStrategyModal({
  open,
  strategyName,
  busy = false,
  previewLoading = false,
  preview,
  error,
  onConfirm,
  onCancel,
}: DeleteStrategyModalProps) {
  const inputId = useId();
  const [typed, setTyped] = useState("");

  const confirmed = typed.trim().toUpperCase() === CONFIRM_PHRASE;

  return (
    <ConfirmationModal
      open={open}
      title="Delete strategy permanently"
      destructive
      busy={busy}
      busyMessage={
        busy
          ? "Deleting strategy data… This can take up to a minute for strategies with long copy history. Please keep this window open."
          : undefined
      }
      busyConfirmLabel="Deleting…"
      error={error}
      confirmLabel="Delete strategy"
      cancelLabel="Keep strategy"
      onCancel={() => {
        setTyped("");
        onCancel();
      }}
      confirmDisabled={
        !confirmed || previewLoading || (preview != null && !preview.can_delete)
      }
      onConfirm={() => {
        if (!confirmed) return;
        onConfirm();
        setTyped("");
      }}
      description={
        <div className="form-stack" style={{ gap: 12 }}>
          <p style={{ margin: 0 }}>
            Strategy <strong>{strategyName}</strong> and all of its history will be removed: copy
            decisions, orders, fills, positions, closed trades, and analytics. This cannot be undone.
          </p>
          {previewLoading ? (
            <p className="muted" style={{ margin: 0, fontSize: 13 }}>
              Checking whether this strategy can be deleted…
            </p>
          ) : null}
          {preview?.blockers?.length ? (
            <ul className="error" style={{ margin: 0, paddingLeft: 18, fontSize: 13 }}>
              {preview.blockers.map((line) => (
                <li key={line} style={{ whiteSpace: "pre-line" }}>
                  {line}
                </li>
              ))}
            </ul>
          ) : null}
          {preview?.warnings?.length ? (
            <ul className="muted" style={{ margin: 0, paddingLeft: 18, fontSize: 13 }}>
              {preview.warnings.map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
          ) : null}
          <p className="muted" style={{ margin: 0, fontSize: 13 }}>
            {preview?.paper_trading
              ? "Disable the strategy in the page header first. Paper delete cancels simulated orders and discards paper positions; remaining strategy cash returns to your account."
              : "The strategy must be disabled with no open positions or active orders. Remaining strategy cash returns to your trading account."}
          </p>
          <p className="muted" style={{ margin: 0, fontSize: 13 }}>
            Source trader events are kept; you can create a new strategy for the same wallet
            afterward.
          </p>
          <label className="form-field" htmlFor={inputId}>
            <span className="muted" style={{ fontSize: 12 }}>
              Type <strong>{CONFIRM_PHRASE}</strong> to confirm
            </span>
            <input
              id={inputId}
              type="text"
              autoComplete="off"
              value={typed}
              disabled={busy}
              onChange={(e) => setTyped(e.target.value)}
              data-testid="delete-strategy-confirm-input"
            />
          </label>
        </div>
      }
    />
  );
}
