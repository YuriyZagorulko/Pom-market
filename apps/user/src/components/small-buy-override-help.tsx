import {
  SMALL_BUY_OVERRIDE_DESCRIPTION,
  smallBuyOverrideMultiplierHint,
  showSmallBuyOverrideSettings,
} from "@/lib/strategy-settings-small-buy";

type Props = {
  sizingMode: string;
  /** When false, only the main description is shown (multiplier hint hidden). */
  showMultiplierHint?: boolean;
};

/** Shared helper copy for Small BUY Override (create wizard + strategy settings). */
export function SmallBuyOverrideHelp({ sizingMode, showMultiplierHint = true }: Props) {
  if (!showSmallBuyOverrideSettings(sizingMode)) {
    return null;
  }
  const hint = showMultiplierHint ? smallBuyOverrideMultiplierHint(sizingMode) : "";
  return (
    <>
      <p
        className="muted"
        data-testid="small-buy-override-description"
        style={{ margin: 0, fontSize: 12 }}
      >
        {SMALL_BUY_OVERRIDE_DESCRIPTION}
      </p>
      {hint ? (
        <p
          className="muted"
          data-testid="small-buy-override-multiplier-hint"
          style={{ margin: "4px 0 0", fontSize: 12 }}
        >
          {hint}
        </p>
      ) : null}
    </>
  );
}