import { describe, expect, it } from "vitest";
import {
  formatCoefficient,
  formatPercent,
  formatSharePriceUsd,
  formatSharesSmart,
  formatUsd,
} from "./display-format";

describe("display-format", () => {
  it("formats USD with commas and trimmed zeros", () => {
    expect(formatUsd("2000")).toBe("$2,000");
    expect(formatUsd("612906.98")).toBe("$612,906.98");
    expect(formatUsd("0.44")).toBe("$0.44");
    expect(formatUsd("13.51")).toBe("$13.51");
  });

  it("formats coefficients and shares without trailing zeros", () => {
    expect(formatCoefficient("10")).toBe("10");
    expect(formatCoefficient("1.5")).toBe("1.5");
    expect(formatCoefficient("0.25")).toBe("0.25");
    expect(formatSharesSmart("100")).toBe("100");
    expect(formatSharesSmart("1.50000000")).toBe("1.5");
  });

  it("formats percents without trailing zeros", () => {
    expect(formatPercent("100")).toBe("100%");
    expect(formatPercent("3.456")).toBe("3.456%");
  });

  it("keeps meaningful precision on share prices", () => {
    expect(formatSharePriceUsd("0.72")).toBe("$0.72");
    expect(formatSharePriceUsd("0.123456")).toBe("$0.123456");
  });

  it("preserves small non-zero USD values", () => {
    expect(formatUsd("0.0015")).toBe("$0.0015");
  });
});
