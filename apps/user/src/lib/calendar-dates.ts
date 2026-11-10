/** UTC calendar helpers for list date filters (must match backend `page.rs`). */

export function utcYmd(d: Date = new Date()): string {
  const y = d.getUTCFullYear();
  const m = String(d.getUTCMonth() + 1).padStart(2, "0");
  const day = String(d.getUTCDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

export function addUtcDays(ymd: string, days: number): string {
  const [y, m, d] = ymd.split("-").map(Number);
  const dt = new Date(Date.UTC(y, m - 1, d + days));
  return utcYmd(dt);
}

export type DatePreset = "today" | "7d" | "30d" | "month" | "all";

export function presetRange(preset: Exclude<DatePreset, "all">): { from: string; to: string } {
  const to = utcYmd();
  switch (preset) {
    case "today":
      return { from: to, to };
    case "7d":
      return { from: addUtcDays(to, -6), to };
    case "30d":
      return { from: addUtcDays(to, -29), to };
    case "month": {
      const now = new Date();
      const from = `${now.getUTCFullYear()}-${String(now.getUTCMonth() + 1).padStart(2, "0")}-01`;
      return { from, to };
    }
  }
}

export function detectPreset(from: string, to: string): DatePreset | null {
  if (!from && !to) return "all";
  if (!from || !to) return null;
  const today = presetRange("today");
  if (from === today.from && to === today.to) return "today";
  const d7 = presetRange("7d");
  if (from === d7.from && to === d7.to) return "7d";
  const d30 = presetRange("30d");
  if (from === d30.from && to === d30.to) return "30d";
  const month = presetRange("month");
  if (from === month.from && to === month.to) return "month";
  return null;
}
