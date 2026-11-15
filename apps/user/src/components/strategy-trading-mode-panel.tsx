"use client";

import { FormEvent, useState } from "react";
import { useToast } from "@/components/toast";
import { useSession } from "@/lib/session";
import {
  buildStrategyOperatingModeBody,
  formatTradingModeApiError,
  LIVE_MODE_PHRASE,
} from "@/lib/strategy-operating-mode";

type Props = {
  strategyId: string;
  strategyEnabled: boolean;
  operatingMode: string;
  liveTradingAvailable: boolean;
  onChanged: () => void;
};

export function StrategyTradingModePanel({
  strategyId,
  strategyEnabled,
  operatingMode,
  liveTradingAvailable,
  onChanged,
}: Props) {
  const { client } = useSession();
  const toast = useToast();
  const [pendingMode, setPendingMode] = useState<"paper_trading" | "live_trading" | null>(null);
  const [phrase, setPhrase] = useState("");
  const [confirmLive, setConfirmLive] = useState(false);
  const [busy, setBusy] = useState(false);

  const current = operatingMode === "live_trading" ? "live_trading" : "paper_trading";

  function openLiveModal() {
    if (!liveTradingAvailable) return;
    setPhrase("");
    setConfirmLive(false);
    setPendingMode("live_trading");
  }

  async function submitMode(event: FormEvent) {
    event.preventDefault();
    if (!pendingMode) return;
    const built = buildStrategyOperatingModeBody({
      operating_mode: pendingMode,
      confirm_live: confirmLive,
      confirmation: phrase,
    });
    if (!built.ok) {
      toast.error(built.error);
      return;
    }
    setBusy(true);
    try {
      await client.request(`/api/v1/strategies/${strategyId}/operating-mode`, {
        method: "POST",
        body: JSON.stringify(built.body),
      });
      toast.success("Trading mode updated");
      setPendingMode(null);
      onChanged();
    } catch (e) {
      const raw = e instanceof Error ? e.message : "Could not update trading mode";
      toast.error(formatTradingModeApiError(raw));
    } finally {
      setBusy(false);
    }
  }

  async function switchToPaper() {
    if (strategyEnabled) {
      toast.error("Disable the strategy before changing trading mode.");
      return;
    }
    setBusy(true);
    try {
      await client.request(`/api/v1/strategies/${strategyId}/operating-mode`, {
        method: "POST",
        body: JSON.stringify({ operating_mode: "paper_trading", confirm_live: false }),
      });
      toast.success("Trading mode set to Paper");
      onChanged();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "Could not update trading mode");
    } finally {
      setBusy(false);
    }
  }

  return (
    <fieldset style={{ border: 0, padding: 0, margin: 0, display: "grid", gap: 12 }}>
      <legend className="muted" style={{ fontSize: 12 }}>
        Trading mode (this strategy)
      </legend>
      <p className="muted" style={{ margin: 0, fontSize: 13 }}>
        Each strategy has its own Paper or Live execution path. Paper funds and positions are not shared with Live.
      </p>
      <div className="filters" style={{ flexWrap: "wrap", alignItems: "center" }}>
        <span>
          Current: <strong>{current === "paper_trading" ? "Paper" : "Live"}</strong>
        </span>
        {current === "paper_trading" ? (
          <button
            type="button"
            className={liveTradingAvailable ? "ghost" : "ghost"}
            disabled={!liveTradingAvailable || strategyEnabled || busy}
            onClick={openLiveModal}
            title={
              liveTradingAvailable
                ? undefined
                : "Live trading is not available yet — execution and accounting are still being prepared."
            }
          >
            Switch to Live…
          </button>
        ) : (
          <button
            type="button"
            className="ghost"
            disabled={strategyEnabled || busy}
            onClick={() => void switchToPaper()}
          >
            Switch to Paper
          </button>
        )}
      </div>
      {!liveTradingAvailable && (
        <p className="warn" style={{ margin: 0, fontSize: 13 }}>
          <strong>Live trading is not available yet.</strong> Real-money copy execution, CLOB reconciliation, and live
          accounting are not production-ready. This strategy will stay on Paper until the platform enables Live.
        </p>
      )}
      {strategyEnabled && (
        <p className="muted" style={{ margin: 0, fontSize: 12 }}>
          Disable the strategy from the header before changing trading mode.
        </p>
      )}

      {pendingMode === "live_trading" && (
        <dialog open className="panel" style={{ maxWidth: 480 }}>
          <form onSubmit={(e) => void submitMode(e)} className="form-stack">
            <h3 style={{ margin: 0 }}>Enable Live trading for this strategy</h3>
            <p className="warn" style={{ margin: 0, fontSize: 13 }}>
              Real USDC on Polymarket will be used. Losses are possible. Paper balances and positions on this strategy
              stay separate and are not converted to Live.
            </p>
            <label style={{ display: "flex", gap: 8, alignItems: "flex-start", fontSize: 13 }}>
              <input
                type="checkbox"
                checked={confirmLive}
                onChange={(e) => setConfirmLive(e.target.checked)}
              />
              I understand this strategy will place real orders when enabled.
            </label>
            <div className="form-field">
              <label htmlFor="live-phrase">Type {LIVE_MODE_PHRASE}</label>
              <input
                id="live-phrase"
                value={phrase}
                onChange={(e) => setPhrase(e.target.value)}
                placeholder={LIVE_MODE_PHRASE}
                autoComplete="off"
              />
            </div>
            <div className="row" style={{ gap: 8 }}>
              <button type="submit" disabled={busy}>
                Confirm Live
              </button>
              <button
                type="button"
                className="ghost"
                disabled={busy}
                onClick={() => setPendingMode(null)}
              >
                Cancel
              </button>
            </div>
          </form>
        </dialog>
      )}
    </fieldset>
  );
}
