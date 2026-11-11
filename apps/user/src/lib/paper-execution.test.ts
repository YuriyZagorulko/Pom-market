import { describe, expect, it } from "vitest";
import { formatPaperExecutionDetail, paperExecutionLabel } from "./paper-execution";

describe("paper-execution", () => {
  it("labels synthetic fills distinctly from CLOB", () => {
    expect(paperExecutionLabel("clob")).toContain("CLOB");
    expect(paperExecutionLabel("synthetic_source_price")).toContain("Paper-only");
  });

  it("formats execution context lines", () => {
    const lines = formatPaperExecutionDetail("synthetic_source_price", {
      source_price: "0.55",
      best_clob_price: "0.93",
      execution_price: "0.55",
      deviation_from_best_clob_bps: 6909,
    });
    expect(lines.some((l) => l.includes("Paper-only"))).toBe(true);
    expect(lines.some((l) => l.includes("6909 bps"))).toBe(true);
  });
});
