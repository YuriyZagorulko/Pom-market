"use client";

import {
  activityPresetRange,
  detectActivityPreset,
  type ActivityDatePreset,
} from "@/lib/strategy-activity-dates";

const PRESETS: Array<{ id: ActivityDatePreset; label: string }> = [
  { id: "today", label: "Today" },
  { id: "yesterday", label: "Yesterday" },
  { id: "7d", label: "Last 7 days" },
  { id: "30d", label: "Last 30 days" },
  { id: "all", label: "All time" },
];

export function StrategyActivityDatePresets({
  from,
  to,
  onChange,
}: {
  from: string;
  to: string;
  onChange: (patch: { from: string | null; to: string | null }) => void;
}) {
  const active = detectActivityPreset(from, to);

  return (
    <div className="date-presets row" role="group" aria-label="Date presets">
      {PRESETS.map((p) => (
        <button
          key={p.id}
          type="button"
          className={`ghost date-preset${active === p.id ? " active" : ""}`}
          onClick={() => {
            if (p.id === "all") {
              onChange({ from: null, to: null });
              return;
            }
            const range = activityPresetRange(p.id);
            onChange({ from: range.from, to: range.to });
          }}
        >
          {p.label}
        </button>
      ))}
    </div>
  );
}
