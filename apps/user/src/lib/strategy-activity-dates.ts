/** Local calendar bounds for strategy activity API (RFC3339 in/out). */

export function localYmd(d: Date = new Date()): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

export function addLocalDays(ymd: string, days: number): string {
  const [y, m, d] = ymd.split("-").map(Number);
  const dt = new Date(y, m - 1, d + days);
  return localYmd(dt);
}

export function localDayStartIso(ymd: string): string {
  const [y, m, d] = ymd.split("-").map(Number);
  return new Date(y, m - 1, d, 0, 0, 0, 0).toISOString();
}

export function localDayEndExclusiveIso(ymd: string): string {
  const [y, m, d] = ymd.split("-").map(Number);
  return new Date(y, m - 1, d + 1, 0, 0, 0, 0).toISOString();
}

/** True when `from`/`to` are YYYY-MM-DD (not full ISO timestamps). */
export function isDateOnly(s: string): boolean {
  return /^\d{4}-\d{2}-\d{2}$/.test(s.trim());
}

export function apiDateBounds(from: string, to: string): { from: string | null; to: string | null } {
  if (!from && !to) return { from: null, to: null };
  const fromOut = from ? (isDateOnly(from) ? localDayStartIso(from) : from) : null;
  const toOut = to ? (isDateOnly(to) ? localDayEndExclusiveIso(to) : to) : null;
  return { from: fromOut, to: toOut };
}

export type ActivityDatePreset = "today" | "yesterday" | "7d" | "30d" | "all";

export function activityPresetRange(preset: Exclude<ActivityDatePreset, "all">): {
  from: string;
  to: string;
} {
  const today = localYmd();
  switch (preset) {
    case "today":
      return { from: today, to: today };
    case "yesterday": {
      const y = addLocalDays(today, -1);
      return { from: y, to: y };
    }
    case "7d":
      return { from: addLocalDays(today, -6), to: today };
    case "30d":
      return { from: addLocalDays(today, -29), to: today };
  }
}

export function detectActivityPreset(from: string, to: string): ActivityDatePreset | null {
  if (!from && !to) return "all";
  if (!from || !to) return null;
  const norm = (s: string) => (isDateOnly(s) ? s : localYmd(new Date(s)));
  const f = norm(from);
  const t = norm(to);
  for (const p of ["today", "yesterday", "7d", "30d"] as const) {
    const r = activityPresetRange(p);
    if (f === r.from && t === r.to) return p;
  }
  return null;
}
