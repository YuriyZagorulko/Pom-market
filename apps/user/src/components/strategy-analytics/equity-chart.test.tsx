import "@testing-library/jest-dom/vitest";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { EquityChart } from "./equity-chart";

const sample = [
  {
    at: "2026-11-10T12:00:00.000Z",
    equity_metric_usd: "1000",
    available_capital_usd: "800",
    deployed_capital_usd: "150",
    reserved_capital_usd: "50",
    total_pnl_usd: "10",
  },
  {
    at: "2026-11-11T15:00:00.000Z",
    equity_metric_usd: "1200",
    available_capital_usd: "900",
    deployed_capital_usd: "250",
    reserved_capital_usd: "50",
    total_pnl_usd: "40",
  },
];

describe("EquityChart", () => {
  it("renders axis labels and legend when capital series exist", () => {
    render(<EquityChart points={sample} period="1M" equityLabel="Strategy equity" />);
    expect(screen.getByText("Amount (USD)")).toBeInTheDocument();
    expect(screen.getByText("Time")).toBeInTheDocument();
    expect(screen.getByText("Available")).toBeInTheDocument();
    expect(screen.getByText("Invested")).toBeInTheDocument();
  });
});
