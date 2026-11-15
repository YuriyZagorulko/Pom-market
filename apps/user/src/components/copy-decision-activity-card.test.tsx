import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { CopyDecisionActivityCard } from "./copy-decision-activity-card";

describe("CopyDecisionActivityCard execution blocks", () => {
  it("shows leader and your execution separately for SELL", () => {
    render(
      <ul>
        <CopyDecisionActivityCard
          row={{
            decision_id: "d1",
            decision_status: "approved",
            execution_status: "executed",
            side: "sell",
            market_title: "Test market",
            outcome: "Yes",
            decided_at: new Date().toISOString(),
            source_price: "0.72",
            source_shares: "100",
            source_notional_usd: "72",
            fill_price: "0.69",
            fill_shares: "25",
            fill_notional_usd: "17.25",
            filled_at: new Date().toISOString(),
            fill_legs: [
              {
                fill_id: "f1",
                filled_at: new Date().toISOString(),
                price: "0.69",
                shares: "25",
                notional_usd: "17.25",
              },
            ],
          }}
          open={false}
          onToggle={() => {}}
        />
      </ul>,
    );
    expect(screen.getByText("Leader execution")).toBeTruthy();
    expect(screen.getByText("Your execution")).toBeTruthy();
    expect(screen.getByText("$72")).toBeTruthy();
    expect(screen.getAllByText("$17.25").length).toBeGreaterThanOrEqual(1);
  });

  it("shows partial close hint when position still open", () => {
    render(
      <ul>
        <CopyDecisionActivityCard
          row={{
            decision_id: "d2",
            decision_status: "approved",
            execution_status: "partial",
            side: "sell",
            market_title: "M",
            outcome: "Yes",
            decided_at: new Date().toISOString(),
            source_price: "0.5",
            source_shares: "50",
            source_notional_usd: "25",
            fill_price: "0.48",
            fill_shares: "20",
            fill_notional_usd: "9.6",
            filled_at: new Date().toISOString(),
            position_open_shares: "30",
          }}
          open={false}
          onToggle={() => {}}
        />
      </ul>,
    );
    expect(screen.getByText(/Partial close/)).toBeTruthy();
  });

  it("renders closed position card", () => {
    render(
      <ul>
        <CopyDecisionActivityCard
          row={{
            decision_id: "c1",
            decision_status: "closed",
            execution_status: "closed",
            side: "closed",
            activity_kind: "closed",
            market_title: "Resolved market",
            outcome: "Yes",
            decided_at: new Date().toISOString(),
            close_reason: "resolution_win",
            close_closed_shares: "40",
            close_avg_entry_price: "0.4",
            close_avg_exit_price: null,
            close_cost_basis_usd: "16",
            close_proceeds_usd: "40",
            close_realized_pnl_usd: "24",
          }}
          open={false}
          onToggle={() => {}}
        />
      </ul>,
    );
    expect(screen.getByText("Closed position")).toBeTruthy();
    expect(screen.getByText("Resolution Win")).toBeTruthy();
  });
});
