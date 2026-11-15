import { describe, expect, it } from "vitest";
import { pct, usd } from "./strategy-analytics-format";

describe("strategy-analytics-format", () => {
  it("formats USD", () => {
    expect(usd("12.5")).toBe("$12.5");
  });
  it("formats percent", () => {
    expect(pct("3.456")).toBe("3.456%");
    expect(pct("100")).toBe("100%");
  });
});
