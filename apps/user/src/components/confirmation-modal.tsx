"use client";

import type { ReactNode } from "react";
import { useEffect, useId, useRef } from "react";

export type ConfirmationModalProps = {
  open: boolean;
  title: string;
  description: ReactNode;
  confirmLabel: string;
  cancelLabel?: string;
  destructive?: boolean;
  busy?: boolean;
  error?: string;
  onConfirm: () => void;
  onCancel: () => void;
  /** When true, confirm button is disabled (in addition to `busy`). */
  confirmDisabled?: boolean;
  /** Shown under the description while `busy` (e.g. long-running deletes). */
  busyMessage?: string;
  busyConfirmLabel?: string;
};

export function ConfirmationModal({
  open,
  title,
  description,
  confirmLabel,
  cancelLabel = "Cancel",
  destructive = false,
  busy = false,
  error,
  onConfirm,
  onCancel,
  confirmDisabled = false,
  busyMessage,
  busyConfirmLabel = "Working…",
}: ConfirmationModalProps) {
  const titleId = useId();
  const descId = useId();
  const cancelRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (open) cancelRef.current?.focus();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, busy, onCancel]);

  if (!open) return null;

  return (
    <div
      className={`modal-backdrop${busy ? " modal-backdrop-busy" : ""}`}
      role="presentation"
      onClick={() => {
        if (!busy) onCancel();
      }}
    >
      <div
        className={`modal-panel${destructive ? " modal-panel-destructive" : ""}`}
        role="dialog"
        aria-modal="true"
        aria-busy={busy}
        aria-labelledby={titleId}
        aria-describedby={descId}
        onClick={(e) => e.stopPropagation()}
      >
        <h2 id={titleId}>{title}</h2>
        <div id={descId} className="modal-description">
          {description}
        </div>
        {busy && busyMessage ? (
          <p className="modal-busy-message" role="status">
            <span className="modal-busy-spinner" aria-hidden />
            {busyMessage}
          </p>
        ) : null}
        {error ? (
          <p className="error modal-error" style={{ whiteSpace: "pre-line" }}>
            {error}
          </p>
        ) : null}
        <div className="modal-actions">
          <button
            type="button"
            className="ghost"
            ref={cancelRef}
            disabled={busy}
            onClick={onCancel}
          >
            {cancelLabel}
          </button>
          <button
            type="button"
            className={destructive ? "modal-confirm-destructive" : undefined}
            disabled={busy || confirmDisabled}
            data-testid="confirmation-modal-confirm"
            onClick={onConfirm}
          >
            {busy ? busyConfirmLabel : confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
