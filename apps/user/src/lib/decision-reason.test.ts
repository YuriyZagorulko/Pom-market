import { describe, expect, it } from "vitest";
import { explainDecisionReason } from "./decision-reason";

describe("explainDecisionReason", () => {
  it("explains position value ratio skip codes", () => {
    expect(explainDecisionReason("source_position_value_unavailable").title).toMatch(/open-position/i);
    expect(explainDecisionReason("source_position_value_stale").title).toMatch(/stale/i);
    expect(explainDecisionReason("source_position_value_zero").title).toMatch(/no open positions/i);
  });
});
