/** X-axis tick labels for equity snapshots (browser local timezone). */

export function niceYTicks(min: number, max: number, targetCount = 5): number[] {
  if (!Number.isFinite(min) || !Number.isFinite(max)) return [0];
  if (min === max) {
    const pad = Math.max(Math.abs(min) * 0.1, 1);
    return [min - pad, min, min + pad];
  }
  const span = max - min;
  const rough = span / Math.max(1, targetCount - 1);
  const mag = 10 ** Math.floor(Math.log10(rough));
  const norm = rough / mag;
  const step =
    norm <= 1 ? mag : norm <= 2 ? 2 * mag : norm <= 5 ? 5 * mag : 10 * mag;
  const start = Math.floor(min / step) * step;
  const ticks: number[] = [];
  for (let v = start; v <= max + step * 0.5; v += step) {
    if (v >= min - step * 0.01) ticks.push(v);
    if (ticks.length > 12) break;
  }
  if (!ticks.length) return [min, max];
  return ticks;
}

const PERIOD_DAY = new Set(["TODAY", "1D"]);
const PERIOD_WEEK = new Set(["1W"]);
const PERIOD_MONTH = new Set(["1M", "30D"]);

export function formatEquityXLabel(iso: string, period: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const code = period.toUpperCase();
  if (PERIOD_DAY.has(code)) {
    return d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  }
  if (PERIOD_WEEK.has(code)) {
    return d.toLocaleString(undefined, {
      weekday: "short",
      month: "short",
      day: "numeric",
    });
  }
  if (PERIOD_MONTH.has(code)) {
    return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  }
  if (code === "ALL" || code === "1Y" || code === "6M" || code === "3M") {
    return d.toLocaleDateString(undefined, { month: "short", year: "2-digit" });
  }
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export function formatEquityTooltipWhen(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    month: "long",
    day: "numeric",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function pickXTickIndices(length: number, maxTicks = 6): number[] {
  if (length <= 0) return [];
  if (length === 1) return [0];
  const count = Math.min(maxTicks, length);
  const out: number[] = [];
  for (let i = 0; i < count; i++) {
    out.push(Math.round((i / (count - 1)) * (length - 1)));
  }
  return [...new Set(out)].sort((a, b) => a - b);
}

export function nearestIndexByFraction(fraction: number, length: number): number {
  if (length <= 1) return 0;
  const f = Math.min(1, Math.max(0, fraction));
  return Math.round(f * (length - 1));
}
