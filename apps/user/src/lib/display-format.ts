function parseNum(value: string | number | null | undefined): number | null {
  if (value === null || value === undefined || value === "") return null;
  const n = typeof value === "number" ? value : Number(value);
  return Number.isFinite(n) ? n : null;
}

export function stripTrailingZeros(formatted: string): string {
  if (!formatted.includes(".")) return formatted;
  return formatted.replace(/(\.\d*?)0+$/, "$1").replace(/\.$/, "");
}

/** USD for UI: thousands separators, up to 2 decimals, no pointless trailing zeros. */
export function formatUsd(value: string | number | null | undefined): string {
  const n = parseNum(value);
  if (n === null) return "—";
  const abs = Math.abs(n);
  const maxFrac = abs > 0 && abs < 0.01 ? 4 : 2;
  const formatted = new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 0,
    maximumFractionDigits: maxFrac,
  }).format(n);
  return stripTrailingZeros(formatted);
}

export function formatNumberCompact(
  value: string | number | null | undefined,
  maxFractionDigits: number,
): string | null {
  const n = parseNum(value);
  if (n === null) return null;
  const formatted = new Intl.NumberFormat("en-US", {
    minimumFractionDigits: 0,
    maximumFractionDigits: maxFractionDigits,
  }).format(n);
  return stripTrailingZeros(formatted);
}

/** Multipliers and dimensionless coefficients. */
export function formatCoefficient(value: string | number | null | undefined): string | null {
  return formatNumberCompact(value, 8);
}

export function formatPercent(value: string | number | null | undefined): string {
  const n = parseNum(value);
  if (n === null) return "—";
  const body = formatNumberCompact(n, 4);
  return body != null ? `${body}%` : "—";
}

export function formatSharesSmart(value: string | number | null | undefined): string | null {
  return formatNumberCompact(value, 4);
}

/** Polymarket share prices in USD. */
export function formatSharePriceUsd(value: string | number | null | undefined): string | null {
  const n = parseNum(value);
  if (n === null) return null;
  const formatted = new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 0,
    maximumFractionDigits: 6,
  }).format(n);
  return stripTrailingZeros(formatted);
}
