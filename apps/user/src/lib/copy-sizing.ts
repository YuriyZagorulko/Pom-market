/** Convert UI percent (1–100) to API proportion coefficient (0–1). */
export function percentToProportion(percent: number): number | null {
  if (!Number.isFinite(percent) || percent <= 0 || percent > 100) return null;
  return Math.round((percent / 100) * 1e8) / 1e8;
}

export function proportionToPercent(proportion: number): number {
  if (!Number.isFinite(proportion)) return 0;
  return Math.round(proportion * 10000) / 100;
}

export const BUDGET_PRESETS = [20, 200, 2000, 20000] as const;
export const COPY_PERCENT_PRESETS = [1, 5, 10, 20, 50] as const;
