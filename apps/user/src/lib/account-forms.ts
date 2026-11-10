/**
 * Shared serialization for Account Edit → API (avoid FormData stringifying all fields).
 */

export type FormResult<T> = { ok: true; body: T } | { ok: false; error: string };

export type AccountRiskPatchBody = {
  max_order_usd: string;
  max_daily_loss_usd: string;
  max_exposure_usd: string;
  max_slippage_bps: number;
  paper_cash_usd: string;
};

export type AccountRiskFormValues = {
  max_order_usd: string;
  max_daily_loss_usd: string;
  max_exposure_usd: string;
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

const USD_FIELDS = [
  "max_order_usd",
  "max_daily_loss_usd",
  "max_exposure_usd",
  "paper_cash_usd",
] as const;

const USD_LABELS: Record<(typeof USD_FIELDS)[number], string> = {
  max_order_usd: "Max order USD",
  max_daily_loss_usd: "Max daily loss USD",
  max_exposure_usd: "Max exposure USD",
  paper_cash_usd: "Paper cash USD",
};

/** Map GET /accounts/{id} risk object into form default strings. */
export function accountRiskFormValues(risk: Record<string, string | number>): AccountRiskFormValues {
  return {
    max_order_usd: String(risk.max_order_usd ?? ""),
    max_daily_loss_usd: String(risk.max_daily_loss_usd ?? ""),
    max_exposure_usd: String(risk.max_exposure_usd ?? ""),
    max_slippage_bps: String(risk.max_slippage_bps ?? ""),
    paper_cash_usd: String(risk.paper_cash_usd ?? ""),
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

/** Build PATCH /api/v1/accounts/{id}/risk body with correct JSON types. */
export function buildAccountRiskPatchBody(form: Record<string, FormDataEntryValue>): FormResult<AccountRiskPatchBody> {
  const body: Partial<AccountRiskPatchBody> = {};
  for (const field of USD_FIELDS) {
    const raw = String(form[field] ?? "");
    const parsed = parseUsdAmount(raw, USD_LABELS[field]);
    if (!parsed.ok) return parsed;
    body[field] = parsed.body;
  }
  const slippage = parseStrictInt(String(form.max_slippage_bps ?? ""), "Max slippage (bps)", {
    min: 0,
    max: 50_000,
  });
  if (!slippage.ok) return slippage;
  body.max_slippage_bps = slippage.body;
  return { ok: true, body: body as AccountRiskPatchBody };
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
