import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { addUtcDays, detectPreset, presetRange, utcYmd } from "./calendar-dates";

describe("utcYmd and addUtcDays", () => {
  it("formats UTC date", () => {
    const d = new Date(Date.UTC(2025, 0, 15, 23, 59, 59));
    expect(utcYmd(d)).toBe("2025-01-15");
  });

  it("rolls month on addUtcDays", () => {
    expect(addUtcDays("2025-01-31", 1)).toBe("2025-02-01");
  });

  it("handles leap day", () => {
    expect(addUtcDays("2024-02-28", 1)).toBe("2024-02-29");
    expect(addUtcDays("2024-02-29", 1)).toBe("2024-03-01");
  });
});

describe("presetRange", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(Date.UTC(2025, 5, 10, 12, 0, 0)));
  });
  afterEach(() => vi.useRealTimers());

  it("today is single UTC day", () => {
    const { from, to } = presetRange("today");
    expect(from).toBe("2025-06-10");
    expect(to).toBe(from);
  });

  it("7d includes six days back plus today", () => {
    const to = utcYmd();
    const { from } = presetRange("7d");
    expect(from).toBe(addUtcDays(to, -6));
  });
});

describe("detectPreset", () => {
  it("recognizes all time", () => {
    expect(detectPreset("", "")).toBe("all");
  });
});
