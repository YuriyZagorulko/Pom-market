"use client";

import { ReactNode } from "react";

/** Visible label for form controls (no help popover). */
export function SimpleFieldLabel({
  htmlFor,
  children,
  optional,
}: {
  htmlFor?: string;
  children: ReactNode;
  optional?: boolean;
}) {
  return (
    <label className="simple-field-label" htmlFor={htmlFor}>
      <span>
        {children}
        {optional ? <span className="muted"> (optional)</span> : null}
      </span>
    </label>
  );
}
