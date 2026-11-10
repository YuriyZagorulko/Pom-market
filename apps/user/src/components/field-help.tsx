"use client";

import { ReactNode, useEffect, useId, useRef, useState } from "react";

type FieldHelpProps = {
  title: string;
  children: ReactNode;
};

export function FieldHelp({ title, children }: FieldHelpProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const panelId = useId();

  useEffect(() => {
    if (!open) return;
    function onPointerDown(event: MouseEvent | TouchEvent) {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (rootRef.current?.contains(target)) return;
      setOpen(false);
    }
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("touchstart", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("touchstart", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  return (
    <div className="field-help" ref={rootRef}>
      <button
        type="button"
        className="field-help-trigger"
        aria-expanded={open}
        aria-controls={panelId}
        aria-label={`Help: ${title}`}
        onClick={() => setOpen((v) => !v)}
      >
        ?
      </button>
      {open && (
        <div className="field-help-popover" id={panelId} role="dialog" aria-label={title}>
          <div className="field-help-popover-head">
            <strong>{title}</strong>
            <button type="button" className="field-help-close ghost" onClick={() => setOpen(false)} aria-label="Close">
              ×
            </button>
          </div>
          <div className="field-help-body">{children}</div>
        </div>
      )}
    </div>
  );
}

export function FieldLabel({
  htmlFor,
  label,
  helpTitle,
  help,
  optional,
}: {
  htmlFor?: string;
  label: string;
  helpTitle: string;
  help: ReactNode;
  optional?: boolean;
}) {
  return (
    <label className="field-label" htmlFor={htmlFor}>
      <span className="field-label-text">
        {label}
        {optional ? <span className="muted"> (optional)</span> : null}
      </span>
      <FieldHelp title={helpTitle}>{help}</FieldHelp>
    </label>
  );
}

export function HelpLink({ href, children }: { href: string; children: ReactNode }) {
  return (
    <a href={href} target="_blank" rel="noopener noreferrer">
      {children}
    </a>
  );
}
