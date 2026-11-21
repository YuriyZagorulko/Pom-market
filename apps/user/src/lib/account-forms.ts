import { formatNumericInputDisplay } from "@pom/format";
import { bpsToPercentInputValue, parseMaxSlippagePercentToBps, MAX_SLIPPAGE_HINT } from "./max-slippage";

export { MAX_SLIPPAGE_HINT };

/**
 * Shared serialization for Account Edit → API (avoid FormData stringifying all fields).
 */

export type FormResult<T> = { ok: true; body: T } | { ok: false; error: string };

export type AccountRiskPatchBody = {
  max_order_usd: string;
  max_daily_loss_usd: string;
  max_slippage_bps: number;
};

export type AccountPaperFundingBody = {
  delta_usd: string;
  reason: string;
  idempotency_key: string;
};

export type AccountRiskFormValues = {
  max_order_usd: string;
  max_daily_loss_usd: string;
  max_slippage_bps: string;
  paper_cash_usd: string;
};

export type AccountModePatchBody = {
  mode: string;
  confirm_live: boolean;
  confirmation?: string;
};

export type AccountRenameBody = {
  name: string;
};

export type AccountTypeValue = "paper" | "wallet";

export type AccountCreateBody = {
  name: string;
  account_type: AccountTypeValue;
  signature_type?: string;
  signer_address?: string;
  funder_address?: string;
  private_key?: string;
};

const RISK_USD_FIELDS = ["max_order_usd", "max_daily_loss_usd"] as const;

const USD_LABELS: Record<(typeof RISK_USD_FIELDS)[number], string> = {
  max_order_usd: "Max order USD",
  max_daily_loss_usd: "Max daily loss USD",
};

/** Map GET /accounts/{id} risk object into form default strings. */
export function accountRiskFormValues(risk: Record<string, string | number>): AccountRiskFormValues {
  return {
    max_order_usd: formatNumericInputDisplay(risk.max_order_usd),
    max_daily_loss_usd: formatNumericInputDisplay(risk.max_daily_loss_usd),
    max_slippage_bps: bpsToPercentInputValue(risk.max_slippage_bps),
    paper_cash_usd: formatNumericInputDisplay(risk.paper_cash_usd),
  };
}

/**
 * Parse a USD amount for the API (rust_decimal accepts JSON strings).
 * Rejects empty, non-numeric, negative, and NaN.
 */
export function parseUsdAmount(raw: string, label: string): FormResult<string> {
  const trimmed = raw.trim();
  if (!trimmed) {
    return { ok: false, error: `${label} is required.` };
  }
  if (!/^-?\d+(\.\d+)?$/.test(trimmed)) {
    return { ok: false, error: `${label} must be a valid number.` };
  }
  const n = Number(trimmed);
  if (!Number.isFinite(n)) {
    return { ok: false, error: `${label} must be a valid number.` };
  }
  if (n < 0) {
    return { ok: false, error: `${label} cannot be negative.` };
  }
  return { ok: true, body: trimmed };
}

/**
 * Strict whole-number field for JSON i32 (e.g. max_slippage_bps).
 * Rejects empty, NaN, fractions, and out-of-range values.
 */
export function parseStrictInt(
  raw: string,
  label: string,
  opts?: { min?: number; max?: number },
): FormResult<number> {
  const trimmed = raw.trim();
  if (!trimmed) {
    return { ok: false, error: `${label} is required.` };
  }
  if (!/^-?\d+$/.test(trimmed)) {
    return { ok: false, error: `${label} must be a whole number (no decimals).` };
  }
  const n = Number(trimmed);
  if (!Number.isFinite(n) || !Number.isInteger(n)) {
    return { ok: false, error: `${label} must be a whole number.` };
  }
  const min = opts?.min ?? 0;
  const max = opts?.max ?? 1_000_000;
  if (n < min || n > max) {
    return { ok: false, error: `${label} must be between ${min} and ${max}.` };
  }
  return { ok: true, body: n };
}

/** Build PATCH /api/v1/accounts/{id}/risk body (limits only; not paper_cash_usd). */
export function buildAccountRiskPatchBody(form: Record<string, FormDataEntryValue>): FormResult<AccountRiskPatchBody> {
  const body: Partial<AccountRiskPatchBody> = {};
  for (const field of RISK_USD_FIELDS) {
    const raw = String(form[field] ?? "");
    const parsed = parseUsdAmount(raw, USD_LABELS[field]);
    if (!parsed.ok) return parsed;
    body[field] = formatNumericInputDisplay(parsed.body);
  }
  const slippage = parseMaxSlippagePercentToBps(String(form.max_slippage_bps ?? ""));
  if (!slippage.ok) return { ok: false, error: slippage.error };
  body.max_slippage_bps = slippage.bps;
  return { ok: true, body: body as AccountRiskPatchBody };
}

/** Paper cash uses POST …/capital/paper-funding (delta), not PATCH risk. */
export function buildPaperFundingRequest(
  currentPaperCash: string | number,
  form: Record<string, FormDataEntryValue>,
  idempotencyKey: string,
): FormResult<AccountPaperFundingBody | null> {
  const raw = String(form.paper_cash_usd ?? "");
  const parsed = parseUsdAmount(raw, "Paper cash USD");
  if (!parsed.ok) return parsed;
  const current = Number(formatNumericInputDisplay(currentPaperCash));
  const target = Number(parsed.body);
  if (!Number.isFinite(current) || !Number.isFinite(target)) {
    return { ok: false, error: "Paper cash USD must be a valid number." };
  }
  const delta = target - current;
  if (delta === 0) {
    return { ok: true, body: null };
  }
  const deltaStr = formatNumericInputDisplay(String(delta));
  return {
    ok: true,
    body: {
      delta_usd: deltaStr,
      reason: "account_ui_paper_funding",
      idempotency_key: idempotencyKey,
    },
  };
}

/** POST /api/v1/accounts — paper needs only name; wallet sends Polymarket fields. */
export function buildCreateAccountBody(
  form: Record<string, FormDataEntryValue>,
): FormResult<AccountCreateBody> {
  const name = String(form.name ?? "").trim();
  if (!name) return { ok: false, error: "Account name is required." };
  const rawType = String(form.account_type ?? "wallet").trim();
  if (rawType !== "paper" && rawType !== "wallet") {
    return { ok: false, error: "Account type must be Paper Trading or Wallet." };
  }
  if (rawType === "paper") {
    return { ok: true, body: { name, account_type: "paper" } };
  }
  const signature_type = String(form.signature_type ?? "").trim();
  if (!signature_type) return { ok: false, error: "Signature type is required for wallet accounts." };
  const signer_address = String(form.signer_address ?? "").trim();
  if (!signer_address) return { ok: false, error: "Signer address is required for wallet accounts." };
  const body: AccountCreateBody = {
    name,
    account_type: "wallet",
    signature_type,
    signer_address,
  };
  const funder = String(form.funder_address ?? "").trim();
  if (funder) body.funder_address = funder;
  const pk = String(form.private_key ?? "").trim();
  if (pk) body.private_key = pk;
  return { ok: true, body };
}

export function buildAccountRenameBody(form: Record<string, FormDataEntryValue>): FormResult<AccountRenameBody> {
  const name = String(form.name ?? "").trim();
  if (!name) return { ok: false, error: "Account name is required." };
  return { ok: true, body: { name } };
}

export function buildAccountModeBody(form: Record<string, FormDataEntryValue>): FormResult<AccountModePatchBody> {
  const mode = String(form.mode ?? "").trim();
  if (!mode) return { ok: false, error: "Operating mode is required." };
  const confirmation = String(form.confirmation ?? "").trim();
  return {
    ok: true,
    body: {
      mode,
      confirm_live: form.confirm_live === "on",
      ...(confirmation ? { confirmation } : {}),
    },
  };
}

/** POST /api/v1/accounts/{id}/credentials/rotate */
export function buildCredentialsRotateBody(
  form: Record<string, FormDataEntryValue>,
  accountName: string,
): Record<string, unknown> {
  const out: Record<string, unknown> = { name: accountName };
  for (const key of ["signature_type", "signer_address", "funder_address", "private_key"] as const) {
    const v = String(form[key] ?? "").trim();
    if (v) out[key] = v;
  }
  return out;
}
