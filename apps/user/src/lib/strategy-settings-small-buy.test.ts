import { describe, expect, it } from "vitest";
import {
  formatSmallBuyThresholdFromApi,
  showSmallBuyOverrideSettings,
  smallBuyFieldsForPatch,
  validateSmallBuyBeforeSave,
} from "./strategy-settings-small-buy";

describe("strategy-settings-small-buy", () => {
  it("shows for PVR and Equal Copy only", () => {
    expect(showSmallBuyOverrideSettings("position_value_ratio")).toBe(true);
    expect(showSmallBuyOverrideSettings("proportional")).toBe(true);
    expect(showSmallBuyOverrideSettings("fixed_amount")).toBe(false);
  });

  it("formats API decimal strings for display", () => {
    expect(formatSmallBuyThresholdFromApi("5.00000000")).toBe("5");
    expect(formatSmallBuyThresholdFromApi("7.5")).toBe("7.5");
    expect(formatSmallBuyThresholdFromApi(null)).toBe("");
  });

  it("builds PATCH fields for PVR and Equal Copy", () => {
    expect(smallBuyFieldsForPatch("fixed_amount", true, "5")).toEqual({});
    expect(smallBuyFieldsForPatch("proportional", false, "5")).toEqual({
      copy_small_buys_1to1: false,
    });
    expect(smallBuyFieldsForPatch("proportional", true, "5")).toEqual({
      copy_small_buys_1to1: true,
      small_buy_threshold_usd: 5,
    });
    expect(smallBuyFieldsForPatch("position_value_ratio", true, "5")).toEqual({
      copy_small_buys_1to1: true,
      small_buy_threshold_usd: 5,
    });
  });

  it("validates threshold when enabled on storable modes", () => {
    expect(validateSmallBuyBeforeSave("fixed_amount", true, "")).toBeNull();
    expect(validateSmallBuyBeforeSave("proportional", false, "")).toBeNull();
    expect(validateSmallBuyBeforeSave("proportional", true, "5")).toBeNull();
    expect(validateSmallBuyBeforeSave("position_value_ratio", true, "5")).toBeNull();
    expect(validateSmallBuyBeforeSave("proportional", true, "")).toMatch(/greater than zero/i);
  });
});
