/** Max copy % in UI (100× notional); create wizard still validates 1–100 separately. */
export const MAX_COPY_PERCENT = 10_000;

/** Convert UI percent to API `proportion` coefficient (20 → 0.2, 150 → 1.5). */
export function percentToProportion(percent: number): number | null {
  if (!Number.isFinite(percent) || percent <= 0 || percent > MAX_COPY_PERCENT) return null;
  return Math.round((percent / 100) * 1e8) / 1e8;
}

export function proportionToPercent(proportion: number): number {
  if (!Number.isFinite(proportion)) return 0;
  return Math.round(proportion * 10000) / 100;
}

export const BUDGET_PRESETS = [20, 200, 2000, 20000] as const;
export const COPY_PERCENT_PRESETS = [1, 5, 10, 20, 50] as const;
