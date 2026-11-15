import { formatPercent, formatUsd } from "./display-format";

export function usd(n: string | number | null | undefined, digits?: number): string {
  if (digits !== undefined) {
    const v = Number(n ?? 0);
    if (!Number.isFinite(v)) return "—";
    return new Intl.NumberFormat("en-US", {
      style: "currency",
      currency: "USD",
      minimumFractionDigits: digits,
      maximumFractionDigits: digits,
    }).format(v);
  }
  return formatUsd(n);
}

export function pct(n: string | number | null | undefined): string {
  return formatPercent(n);
}

export function pnlClass(n: number): string {
  if (n > 0) return "positive";
  if (n < 0) return "negative";
  return "muted";
}
