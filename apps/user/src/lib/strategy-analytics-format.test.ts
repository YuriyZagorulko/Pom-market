import { describe, expect, it } from "vitest";
import { pct, usd } from "./strategy-analytics-format";

describe("strategy-analytics-format", () => {
  it("formats USD", () => {
    expect(usd("12.5")).toMatch(/\$12\.50/);
  });
  it("formats percent", () => {
    expect(pct("3.456")).toBe("3.46%");
  });
});
