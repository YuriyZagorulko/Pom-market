/** Modes shown in create wizard and strategy settings (save). */
export const USER_SIZING_MODES = [
  "position_value_ratio",
  "fixed_amount",
  "proportional",
] as const;

export type UserSizingMode = (typeof USER_SIZING_MODES)[number];

export const LEGACY_SIZING_MODES = ["portfolio_ratio"] as const;

export type SizingMode = UserSizingMode | (typeof LEGACY_SIZING_MODES)[number];

export function isUserSizingMode(mode: string): mode is UserSizingMode {
  return (USER_SIZING_MODES as readonly string[]).includes(mode);
}

export function isLegacySizingMode(mode: string): boolean {
  return (LEGACY_SIZING_MODES as readonly string[]).includes(mode);
}

export function sizingModeLabel(mode: string): string {
  switch (mode) {
    case "position_value_ratio":
      return "Position Value Ratio";
    case "fixed_amount":
      return "Fixed USD per trade";
    case "proportional":
      return "Percent of source trade";
    case "portfolio_ratio":
      return "Portfolio-relative (legacy)";
    case "multiplier":
      return "Multiplier (legacy)";
    default:
      return mode;
  }
}

export type SizingModeOption = { value: string; label: string; hint: string };

/** Options for strategy settings: three user modes plus legacy row when needed. */
export function settingsSizingSelectOptions(currentMode: string): SizingModeOption[] {
  const base = SIZING_MODE_OPTIONS.map((o) => ({
    value: o.value,
    label: o.label,
    hint: o.hint,
  }));
  if (!base.some((o) => o.value === currentMode)) {
    base.push({
      value: currentMode,
      label: sizingModeLabel(currentMode),
      hint: "Deprecated sizing mode; behavior unchanged until you pick a supported mode.",
    });
  }
  return base;
}

export const SIZING_MODE_OPTIONS: { value: UserSizingMode; label: string; hint: string }[] = [
  {
    value: "position_value_ratio",
    label: "Position Value Ratio (recommended)",
    hint:
      "target ≈ allocation × (source BUY ÷ leader open-position value) × trade multiplier. Uses open-position market value, not wallet equity.",
  },
  {
    value: "fixed_amount",
    label: "Fixed USD per trade",
    hint: "Same USD on every copied BUY (then min/max order, exposure, and available capital apply).",
  },
  {
    value: "proportional",
    label: "Percent of source trade notional",
    hint: "target = source trade notional × your percent ÷ 100 (e.g. $100 source × 20% → $20).",
  },
];
