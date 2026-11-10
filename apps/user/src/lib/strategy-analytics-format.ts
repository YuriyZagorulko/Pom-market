export function usd(n: string | number | null | undefined, digits = 2): string {
  const v = Number(n ?? 0);
  if (!Number.isFinite(v)) return "—";
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  }).format(v);
}

export function pct(n: string | number | null | undefined): string {
  const v = Number(n ?? 0);
  if (!Number.isFinite(v)) return "—";
  return `${v.toFixed(2)}%`;
}

export function pnlClass(n: number): string {
  if (n > 0) return "positive";
  if (n < 0) return "negative";
  return "muted";
}
