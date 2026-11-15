import { formatNumericInputDisplay } from "@pom/format";

export const SMALL_BUY_OVERRIDE_DESCRIPTION =
  "BUY trades below the threshold use source trade notional × trade multiplier instead of normal sizing.";

export function formatSmallBuyThresholdFromApi(value: unknown): string {
  if (value == null || value === "") return "";
  return formatNumericInputDisplay(String(value));
}

export function showSmallBuyOverrideSettings(sizingMode: string): boolean {
  return sizingMode === "position_value_ratio" || sizingMode === "proportional";
}

/** @deprecated use showSmallBuyOverrideSettings */
export const showSmallBuy1to1Settings = showSmallBuyOverrideSettings;

export function smallBuySettingsStorable(sizingMode: string): boolean {
  return showSmallBuyOverrideSettings(sizingMode);
}

/** PATCH fragment for small BUY override (PVR and Equal Copy; omit on Fixed USD to preserve DB). */
export function smallBuyFieldsForPatch(
  sizingMode: string,
  enabled: boolean,
  thresholdInput: string,
): { copy_small_buys_1to1?: boolean; small_buy_threshold_usd?: number } {
  if (!smallBuySettingsStorable(sizingMode)) {
    return {};
  }
  const body: { copy_small_buys_1to1: boolean; small_buy_threshold_usd?: number } = {
    copy_small_buys_1to1: enabled,
  };
  const trimmed = thresholdInput.trim();
  if (enabled && trimmed) {
    body.small_buy_threshold_usd = Number(trimmed);
  }
  return body;
}

export function validateSmallBuyBeforeSave(
  sizingMode: string,
  enabled: boolean,
  thresholdInput: string,
): string | null {
  if (!smallBuySettingsStorable(sizingMode) || !enabled) return null;
  const n = Number(thresholdInput.trim());
  if (!Number.isFinite(n) || n <= 0) {
    return "Small BUY threshold (USD) must be greater than zero when Small BUY Override is enabled.";
  }
  return null;
}
