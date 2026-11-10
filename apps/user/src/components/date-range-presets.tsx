"use client";

import { detectPreset, presetRange, type DatePreset } from "@/lib/calendar-dates";

const PRESETS: Array<{ id: Exclude<DatePreset, "all"> | "all"; label: string }> = [
  { id: "today", label: "Today" },
  { id: "7d", label: "7 days" },
  { id: "30d", label: "30 days" },
  { id: "month", label: "This month" },
  { id: "all", label: "All time" },
];

export function DateRangePresets({
  from,
  to,
  onChange,
}: {
  from: string;
  to: string;
  onChange: (patch: { from: string | null; to: string | null }) => void;
}) {
  const active = detectPreset(from, to);

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
            const range = presetRange(p.id);
            onChange({ from: range.from, to: range.to });
          }}
        >
          {p.label}
        </button>
      ))}
    </div>
  );
}
