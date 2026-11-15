import { describe, expect, it } from "vitest";
import {
  activityPresetRange,
  apiDateBounds,
  detectActivityPreset,
  localDayEndExclusiveIso,
  localDayStartIso,
} from "./strategy-activity-dates";

describe("strategy activity local dates", () => {
  it("converts local day to bounded ISO range", () => {
    const { from, to } = apiDateBounds("2026-03-15", "2026-03-15");
    expect(from).toBe(localDayStartIso("2026-03-15"));
    expect(to).toBe(localDayEndExclusiveIso("2026-03-15"));
  });

  it("detects today preset", () => {
    const r = activityPresetRange("today");
    expect(detectActivityPreset(r.from, r.to)).toBe("today");
  });

  it("detects yesterday preset", () => {
    const r = activityPresetRange("yesterday");
    expect(detectActivityPreset(r.from, r.to)).toBe("yesterday");
  });
});
