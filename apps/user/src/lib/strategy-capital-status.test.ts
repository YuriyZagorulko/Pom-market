import { describe, expect, it } from "vitest";
import { capitalFundingBadgeClass, capitalFundingLabel } from "./strategy-capital-status";

describe("strategy capital status labels", () => {
  it("maps ledger funding states to user-facing labels", () => {
    expect(capitalFundingLabel("not_funded")).toBe("Not funded");
    expect(capitalFundingLabel("partially_funded")).toBe("Partially funded");
    expect(capitalFundingLabel("funded")).toBe("Funded");
  });

  it("assigns distinct badge classes per state", () => {
    expect(capitalFundingBadgeClass("funded")).toContain("capital-funded");
    expect(capitalFundingBadgeClass("not_funded")).toContain("capital-not-funded");
    expect(capitalFundingBadgeClass("partially_funded")).toContain("capital-partial");
  });
});
