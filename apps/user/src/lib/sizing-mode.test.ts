import { describe, expect, it } from "vitest";
import {
  isLegacySizingMode,
  isUserSizingMode,
  settingsSizingSelectOptions,
  SIZING_MODE_OPTIONS,
  sizingModeLabel,
  USER_SIZING_MODES,
} from "./sizing-mode";

describe("sizing-mode", () => {
  it("exposes exactly three user modes", () => {
    expect(USER_SIZING_MODES).toEqual(["position_value_ratio", "fixed_amount", "proportional"]);
    expect(SIZING_MODE_OPTIONS).toHaveLength(3);
  });

  it("labels legacy portfolio_ratio", () => {
    expect(isLegacySizingMode("portfolio_ratio")).toBe(true);
    expect(isUserSizingMode("portfolio_ratio")).toBe(false);
    expect(sizingModeLabel("portfolio_ratio")).toMatch(/legacy/i);
  });

  it("adds legacy option in settings without duplicating user modes", () => {
    expect(settingsSizingSelectOptions("position_value_ratio")).toHaveLength(3);
    const legacy = settingsSizingSelectOptions("portfolio_ratio");
    expect(legacy).toHaveLength(4);
    expect(legacy.map((o) => o.value)).toContain("portfolio_ratio");
    expect(legacy.filter((o) => o.value === "position_value_ratio")).toHaveLength(1);
  });
});
