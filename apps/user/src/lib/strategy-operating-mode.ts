export const LIVE_MODE_PHRASE = "ENABLE LIVE TRADING";
export const ACCOUNT_CREDENTIALS_REQUIRED_CODE = "account_credentials_required";

export function formatTradingModeApiError(raw: string): string {
  if (raw.includes(ACCOUNT_CREDENTIALS_REQUIRED_CODE)) {
    return "This account has no stored private key. Open the linked trading account → Polymarket credentials, add your key, then try Live again.";
  }
  if (raw.includes("live_trading_not_ready")) {
    return "Live trading is not enabled on this platform yet.";
  }
  return raw;
}

export type StrategyOperatingModeBody = {
  operating_mode: "paper_trading" | "live_trading";
  confirm_live: boolean;
  confirmation?: string;
};

export type FormResult<T> = { ok: true; body: T } | { ok: false; error: string };

export function buildStrategyOperatingModeBody(input: {
  operating_mode: string;
  confirm_live: boolean;
  confirmation?: string;
}): FormResult<StrategyOperatingModeBody> {
  if (input.operating_mode !== "paper_trading" && input.operating_mode !== "live_trading") {
    return { ok: false, error: "Invalid trading mode." };
  }
  if (input.operating_mode === "live_trading") {
    if (!input.confirm_live) {
      return { ok: false, error: "Confirm that you understand live trading risks." };
    }
    if (input.confirmation?.trim() !== LIVE_MODE_PHRASE) {
      return { ok: false, error: `Type exactly: ${LIVE_MODE_PHRASE}` };
    }
    return {
      ok: true,
      body: {
        operating_mode: "live_trading",
        confirm_live: true,
        confirmation: LIVE_MODE_PHRASE,
      },
    };
  }
  return {
    ok: true,
    body: { operating_mode: "paper_trading", confirm_live: false },
  };
}
