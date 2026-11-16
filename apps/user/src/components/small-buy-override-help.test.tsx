import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach } from "vitest";
import { describe, expect, it } from "vitest";
import { SmallBuyOverrideHelp } from "./small-buy-override-help";
import { SMALL_BUY_OVERRIDE_DESCRIPTION } from "@/lib/strategy-settings-small-buy";

afterEach(() => cleanup());

describe("SmallBuyOverrideHelp", () => {
  it("shows at or below threshold description", () => {
    render(<SmallBuyOverrideHelp sizingMode="proportional" showMultiplierHint={false} />);
    expect(screen.getByTestId("small-buy-override-description")).toHaveTextContent(
      SMALL_BUY_OVERRIDE_DESCRIPTION,
    );
    expect(SMALL_BUY_OVERRIDE_DESCRIPTION).toMatch(/at or below the threshold/i);
  });

  it("shows Copy % hint for Equal Copy when enabled", () => {
    render(<SmallBuyOverrideHelp sizingMode="proportional" showMultiplierHint />);
    expect(screen.getByTestId("small-buy-override-multiplier-hint")).toHaveTextContent(/Copy %/i);
    expect(screen.getByTestId("small-buy-override-multiplier-hint")).toHaveTextContent(/50%/);
  });

  it("shows PVR trade multiplier hint", () => {
    render(<SmallBuyOverrideHelp sizingMode="position_value_ratio" showMultiplierHint />);
    const hint = screen.getByTestId("small-buy-override-multiplier-hint");
    expect(hint).toHaveTextContent(/PVR trade multiplier/i);
    expect(hint).toHaveTextContent(/1× if not set/i);
  });

  it("renders nothing for fixed_amount", () => {
    const { container } = render(<SmallBuyOverrideHelp sizingMode="fixed_amount" />);
    expect(container.firstChild).toBeNull();
  });
});