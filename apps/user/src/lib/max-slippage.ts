/**
 * Max slippage is stored as integer BPS in API/DB. UI shows percent (1 bps = 0.01%).
 */

export const MAX_SLIPPAGE_BPS = 50_000;
export const MAX_SLIPPAGE_HINT =
  "Maximum allowed price deviation from the leader's execution price.";

export function bpsToPercentNumber(bps: number): number {
  if (!Number.isFinite(bps)) return 0;
  return bps / 100;
}

/** Editable field value from stored BPS (e.g. 150 → "1.5"). */
export function bpsToPercentInputValue(bps: number | string | null | undefined): string {
  const n = typeof bps === "string" ? Number(bps) : Number(bps ?? 0);
  if (!Number.isFinite(n)) return "";
  const pct = n / 100;
  const rounded = Math.round(pct * 1000) / 1000;
  return String(rounded);
}

/** Display with % suffix (e.g. 150 → "1.5%"). */
export function formatMaxSlippagePercent(bps: number | string | null | undefined): string {
  const raw = bpsToPercentInputValue(bps);
  if (!raw) return "—";
  return `${raw}%`;
}

export type SlippageParseResult =
  | { ok: true; bps: number }
  | { ok: false; error: string };

/**
 * Parse user percent input to BPS. Accepts "1.5", "1.5%", or legacy whole BPS "150".
 */
export function parseMaxSlippagePercentToBps(raw: string, label = "Max slippage"): SlippageParseResult {
  const trimmed = raw.trim().replace(/%$/, "").trim();
  if (!trimmed) {
    return { ok: false, error: `${label} is required.` };
  }
  if (!/^-?\d+(\.\d+)?$/.test(trimmed)) {
    return { ok: false, error: `${label} must be a valid percent (e.g. 1.5).` };
  }
  const n = Number(trimmed);
  if (!Number.isFinite(n) || n < 0) {
    return { ok: false, error: `${label} cannot be negative.` };
  }
  const bps = Math.round(n * 100);
  if (!Number.isInteger(bps) || bps < 0 || bps > MAX_SLIPPAGE_BPS) {
    return {
      ok: false,
      error: `${label} must be between 0% and ${MAX_SLIPPAGE_BPS / 100}%.`,
    };
  }
  return { ok: true, bps };
}