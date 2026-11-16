import { formatNumericInputDisplay } from "@pom/format";

export const SMALL_BUY_OVERRIDE_DESCRIPTION =
  "BUY trades at or below the threshold use source trade notional × trade multiplier instead of normal sizing.";

/** How trade multiplier is chosen for Small BUY Override per sizing mode (matches backend). */
export function smallBuyOverrideMultiplierHint(sizingMode: string): string {
  switch (sizingMode) {
    case "proportional":
      return "Small BUY Override uses your Trade multiplier (Copy %). Example: leader BUY $4, threshold $5, multiplier 50% → follower target $2.";
    case "position_value_ratio":
      return "Small BUY Override uses your PVR trade multiplier (1× if not set). Example: threshold $5, multiplier 0.5, leader BUY $4 → follower target $2.";
    case "fixed_amount":
      return "For small BUY trades, Fixed Amount sizing is bypassed. The source trade notional is copied at 1×, subject to strategy limits.";
    case "portfolio_ratio":
      return "For small BUY trades, Portfolio Ratio sizing is bypassed. The source trade notional is copied at 1×, subject to strategy limits.";
    default:
      return "";
  }
}

/** Shown under Fixed / legacy Portfolio sizing (override is not configurable in UI for these modes). */
export function smallBuyNoteForNonConfigurableModes(sizingMode: string): string | null {
  if (sizingMode === "fixed_amount" || sizingMode === "portfolio_ratio") {
    return smallBuyOverrideMultiplierHint(sizingMode);
  }
  return null;
}

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
