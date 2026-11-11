import { describe, expect, it } from "vitest";
import { capitalSyncSuccessMessage } from "./strategy-capital-sync";

describe("capitalSyncSuccessMessage", () => {
  it("describes a new allocation", () => {
    const msg = capitalSyncSuccessMessage({
      configured_capital_usd: "2000",
      funded_capital_usd: "2000",
      available_cash_usd: "2000",
      delta_applied_usd: "2000",
      bootstrap_quality: "migrated",
    });
    expect(msg).toContain("2,000");
    expect(msg).toContain("Moved");
  });

  it("describes idempotent no-op", () => {
    const msg = capitalSyncSuccessMessage({
      configured_capital_usd: "2000",
      funded_capital_usd: "2000",
      available_cash_usd: "1800",
      delta_applied_usd: "0",
      bootstrap_quality: "migrated",
    });
    expect(msg).toContain("already funded");
  });
});
