import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi, beforeEach } from "vitest";
import { StrategySettingsForm } from "./strategy-detail-view";

const request = vi.fn();
const toastError = vi.fn();

vi.mock("@/lib/session", () => ({
  useSession: () => ({ client: { request } }),
}));

vi.mock("@/components/toast", () => ({
  useToast: () => ({ error: toastError, success: vi.fn(), info: vi.fn() }),
}));

const baseData: Record<string, unknown> = {
  sizing_mode: "position_value_ratio",
  allocation_mode: "fixed_usd",
  strategy_budget_usd: "2000",
  max_position_usd: "50",
  max_open_positions: 10,
  min_order_usd: "1",
  max_slippage_bps: 150,
  cooldown_seconds: 0,
  copy_buy: true,
  copy_sell: true,
  allowed_markets: [],
  copy_small_buys_1to1: false,
  small_buy_threshold_usd: null,
};

function withinSettingsForm() {
  return within(document.querySelector("form.settings-form") as HTMLFormElement);
}

describe("StrategySettingsForm Small BUY Override", () => {
  beforeEach(() => {
    cleanup();
    request.mockReset();
    toastError.mockReset();
    vi.stubGlobal("confirm", () => true);
  });

  it("shows checkbox for PVR and Equal Copy, not Fixed USD", () => {
    const { rerender } = render(
      <StrategySettingsForm id="s1" data={baseData} strategyEnabled={false} onSaved={() => {}} />,
    );
    expect(withinSettingsForm().getByTestId("copy-small-buys-1to1")).toBeInTheDocument();

    rerender(
      <StrategySettingsForm
        id="s1"
        data={{ ...baseData, sizing_mode: "proportional", proportion: "0.5" }}
        strategyEnabled={false}
        onSaved={() => {}}
      />,
    );
    expect(withinSettingsForm().getByTestId("copy-small-buys-1to1")).toBeInTheDocument();

    rerender(
      <StrategySettingsForm
        id="s1"
        data={{ ...baseData, sizing_mode: "fixed_amount" }}
        strategyEnabled={false}
        onSaved={() => {}}
      />,
    );
    expect(withinSettingsForm().queryByTestId("copy-small-buys-1to1")).not.toBeInTheDocument();
  });

  it("shows threshold on Equal Copy when override enabled", () => {
    render(
      <StrategySettingsForm
        id="s1"
        data={{ ...baseData, sizing_mode: "proportional", proportion: "1" }}
        strategyEnabled={false}
        onSaved={() => {}}
      />,
    );
    const form = withinSettingsForm();
    fireEvent.click(form.getByTestId("copy-small-buys-1to1"));
    expect(form.getByTestId("small-buy-threshold-usd")).toBeInTheDocument();
  });

  it("shows threshold when checkbox enabled", () => {
    render(<StrategySettingsForm id="s1" data={baseData} strategyEnabled={false} onSaved={() => {}} />);
    const f = withinSettingsForm();
    expect(f.queryByTestId("small-buy-threshold-usd")).not.toBeInTheDocument();
    fireEvent.click(f.getByTestId("copy-small-buys-1to1"));
    expect(f.getByTestId("small-buy-threshold-usd")).toBeInTheDocument();
  });

  it("loads saved values from API-shaped data", () => {
    render(
      <StrategySettingsForm
        id="s1"
        data={{
          ...baseData,
          copy_small_buys_1to1: true,
          small_buy_threshold_usd: "5.00000000",
        }}
        strategyEnabled={false}
        onSaved={() => {}}
      />,
    );
    const f = withinSettingsForm();
    expect(f.getByTestId("copy-small-buys-1to1")).toBeChecked();
    expect(f.getByTestId("small-buy-threshold-usd")).toHaveValue("5");
  });

  it("preserves small-buy state when switching sizing mode away and back", () => {
    render(
      <StrategySettingsForm
        id="s1"
        data={{
          ...baseData,
          copy_small_buys_1to1: true,
          small_buy_threshold_usd: "10",
        }}
        strategyEnabled={false}
        onSaved={() => {}}
      />,
    );
    const f = withinSettingsForm();
    expect(f.getByTestId("small-buy-threshold-usd")).toHaveValue("10");
    fireEvent.change(f.getByLabelText(/Sizing mode/i), { target: { value: "fixed_amount" } });
    expect(f.queryByTestId("small-buy-threshold-usd")).not.toBeInTheDocument();
    fireEvent.change(f.getByLabelText(/Sizing mode/i), { target: { value: "proportional" } });
    expect(f.getByTestId("copy-small-buys-1to1")).toBeChecked();
    expect(f.getByTestId("small-buy-threshold-usd")).toHaveValue("10");
    fireEvent.change(f.getByLabelText(/Sizing mode/i), { target: { value: "position_value_ratio" } });
    expect(f.getByTestId("copy-small-buys-1to1")).toBeChecked();
    expect(f.getByTestId("small-buy-threshold-usd")).toHaveValue("10");
  });

  it("PATCHes small-buy fields on Equal Copy save", async () => {
    request.mockResolvedValue({ ok: true });
    render(
      <StrategySettingsForm
        id="strat-ec"
        data={{ ...baseData, sizing_mode: "proportional", proportion: "0.5" }}
        strategyEnabled={false}
        onSaved={() => {}}
      />,
    );
    const f = withinSettingsForm();
    fireEvent.click(f.getByTestId("copy-small-buys-1to1"));
    fireEvent.change(f.getByTestId("small-buy-threshold-usd"), { target: { value: "7" } });
    fireEvent.click(f.getByRole("button", { name: /Save changes/i }));
    await waitFor(() => expect(request).toHaveBeenCalledTimes(1));
    const parsed = JSON.parse((request.mock.calls[0] as [string, { body: string }])[1].body) as Record<
      string,
      unknown
    >;
    expect(parsed.sizing_mode).toBe("proportional");
    expect(parsed.copy_small_buys_1to1).toBe(true);
    expect(parsed.small_buy_threshold_usd).toBe(7);
  });

  it("omits small-buy fields from PATCH when Fixed USD", async () => {
    request.mockResolvedValue({ ok: true });
    render(
      <StrategySettingsForm
        id="strat-fixed"
        data={{
          ...baseData,
          sizing_mode: "fixed_amount",
          fixed_amount_usd: "25",
          copy_small_buys_1to1: true,
          small_buy_threshold_usd: "5",
        }}
        strategyEnabled={false}
        onSaved={() => {}}
      />,
    );
    const f = withinSettingsForm();
    fireEvent.click(f.getByRole("button", { name: /Save changes/i }));
    await waitFor(() => expect(request).toHaveBeenCalledTimes(1));
    const parsed = JSON.parse((request.mock.calls[0] as [string, { body: string }])[1].body) as Record<
      string,
      unknown
    >;
    expect(parsed.copy_small_buys_1to1).toBeUndefined();
    expect(parsed.small_buy_threshold_usd).toBeUndefined();
  });

  it("PATCHes on Save only with small-buy fields", async () => {
    request.mockResolvedValue({ ok: true });
    const onSaved = vi.fn();
    render(
      <StrategySettingsForm id="strat-uuid" data={baseData} strategyEnabled={false} onSaved={onSaved} />,
    );
    const f = withinSettingsForm();
    fireEvent.click(f.getByTestId("copy-small-buys-1to1"));
    fireEvent.change(f.getByTestId("small-buy-threshold-usd"), { target: { value: "5" } });
    fireEvent.click(f.getByRole("button", { name: /Save changes/i }));
    await waitFor(() => expect(request).toHaveBeenCalledTimes(1));
    const [path, opts] = request.mock.calls[0] as [string, { method: string; body: string }];
    expect(path).toBe("/api/v1/strategies/strat-uuid");
    expect(opts.method).toBe("PATCH");
    const parsed = JSON.parse(opts.body) as Record<string, unknown>;
    expect(parsed.copy_small_buys_1to1).toBe(true);
    expect(parsed.small_buy_threshold_usd).toBe(5);
    expect(onSaved).toHaveBeenCalled();
  });

  it("rehydrates from updated data after parent reload (remount key)", () => {
    const { rerender } = render(
      <StrategySettingsForm
        key="v1"
        id="s1"
        data={{
          ...baseData,
          copy_small_buys_1to1: true,
          small_buy_threshold_usd: "5",
        }}
        strategyEnabled={false}
        onSaved={() => {}}
      />,
    );
    expect(withinSettingsForm().getByTestId("small-buy-threshold-usd")).toHaveValue("5");
    rerender(
      <StrategySettingsForm
        key="v2"
        id="s1"
        data={{
          ...baseData,
          copy_small_buys_1to1: true,
          small_buy_threshold_usd: "7.50000000",
        }}
        strategyEnabled={false}
        onSaved={() => {}}
      />,
    );
    expect(withinSettingsForm().getByTestId("small-buy-threshold-usd")).toHaveValue("7.5");
  });
});
