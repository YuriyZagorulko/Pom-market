export type OverviewResponse = {
  users?: number;
  accounts?: number;
  source_traders?: number;
  source_events?: number;
  errors_24h?: number;
  kill_switch?: { enabled?: boolean } | string;
};

const LABELS: Record<string, string> = {
  users: "Users",
  accounts: "Trading accounts",
  source_traders: "Source traders",
  source_events: "Source events",
  errors_24h: "Errors (24h)",
};

export function overviewMetricLabel(key: string): string {
  return LABELS[key] ?? key.replace(/_/g, " ");
}

export function parseKillSwitch(value: unknown): boolean | null {
  if (value == null) return null;
  if (typeof value === "object" && value !== null && "enabled" in value) {
    return Boolean((value as { enabled?: boolean }).enabled);
  }
  if (typeof value === "string") {
    try {
      const parsed = JSON.parse(value) as { enabled?: boolean };
      return Boolean(parsed.enabled);
    } catch {
      return null;
    }
  }
  return null;
}

export function countMetrics(data: OverviewResponse): { key: string; label: string; value: number }[] {
  return (["users", "accounts", "source_traders", "source_events", "errors_24h"] as const)
    .map((key) => ({
      key,
      label: overviewMetricLabel(key),
      value: Number(data[key] ?? 0),
    }));
}
