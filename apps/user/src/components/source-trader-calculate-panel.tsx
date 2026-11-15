"use client";

import { FormEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useSession } from "@/lib/session";
import {
  requestWithTimeout,
  SOURCE_TRADER_CALCULATE_TIMEOUT_MS,
} from "@/lib/request-with-timeout";
import { pct, usd } from "@/lib/strategy-analytics-format";

type Period = "7D" | "30D" | "90D" | "ALL" | "custom";

const PERIODS: Array<{ id: Period; label: string }> = [
  { id: "7D", label: "7 Days" },
  { id: "30D", label: "30 Days" },
  { id: "90D", label: "90 Days" },
  { id: "ALL", label: "All Time" },
  { id: "custom", label: "Custom" },
];

type CalcParams = {
  capital: string;
  period: Period;
  customFrom: string;
  customTo: string;
  targetPct: number;
};

type CalcResponse = Record<string, unknown>;

type SavedCalculate = {
  savedAt: string;
  params: CalcParams;
  response: CalcResponse;
  cacheFingerprint: string;
};

function num(v: unknown): number | null {
  if (v == null) return null;
  const n = Number(v);
  return Number.isFinite(n) ? n : null;
}

function str(v: unknown): string {
  return v == null ? "" : String(v);
}

function covPctFrac(v: unknown): string {
  const n = num(v);
  if (n == null) return "—";
  return pct(n * 100);
}

function storageKey(traderId: string) {
  return `pom-source-trader-calculate:${traderId}`;
}

function paramsEqual(a: CalcParams, b: CalcParams): boolean {
  return (
    a.capital === b.capital &&
    a.period === b.period &&
    a.customFrom === b.customFrom &&
    a.customTo === b.customTo &&
    a.targetPct === b.targetPct
  );
}

function formatCoeff(v: unknown): string {
  const n = num(v);
  if (n == null) return "—";
  return `${n.toFixed(4)}x`;
}

function formatDivisor(v: unknown): string {
  const n = num(v);
  if (n == null) return "—";
  return n >= 10 ? n.toFixed(1) : n.toFixed(2);
}

function validateTarget(raw: string): number | null {
  const n = Number(raw);
  if (!Number.isFinite(n) || n < 1 || n > 100) return null;
  return Math.round(n);
}

function parseCalcAt(v: unknown): string {
  const s = str(v);
  if (!s) return "";
  const d = new Date(s);
  if (Number.isNaN(d.getTime())) return s;
  return d.toLocaleString();
}

export function SourceTraderCalculatePanel({
  traderId,
  alias,
}: {
  traderId: string;
  alias: string;
}) {
  const { client } = useSession();
  const [capital, setCapital] = useState("8000");
  const [period, setPeriod] = useState<Period>("30D");
  const [customFrom, setCustomFrom] = useState("");
  const [customTo, setCustomTo] = useState("");
  const [targetInput, setTargetInput] = useState("100");
  const [loading, setLoading] = useState(false);
  const calcInFlight = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [data, setData] = useState<CalcResponse | null>(null);
  const [savedParams, setSavedParams] = useState<CalcParams | null>(null);
  const [cacheStale, setCacheStale] = useState(false);
  const [metaChecked, setMetaChecked] = useState(false);

  const currentParams = useMemo<CalcParams>(
    () => ({
      capital,
      period,
      customFrom,
      customTo,
      targetPct: validateTarget(targetInput) ?? 100,
    }),
    [capital, period, customFrom, customTo, targetInput],
  );

  const paramsDirty = data != null && savedParams != null && !paramsEqual(currentParams, savedParams);

  const targetValid = validateTarget(targetInput);

  const body = useMemo(() => {
    const cap = Number(capital);
    if (!Number.isFinite(cap) || cap < 0) return null;
    if (targetValid == null) return null;
    return {
      available_capital_usd: cap,
      period: period === "custom" ? undefined : period,
      from:
        period === "custom" && customFrom
          ? new Date(customFrom).toISOString()
          : undefined,
      to:
        period === "custom" && customTo ? new Date(customTo).toISOString() : undefined,
      target_coverage_percent: targetValid,
    };
  }, [capital, period, customFrom, customTo, targetValid]);

  const persist = useCallback(
    (response: CalcResponse, params: CalcParams) => {
      const fp = str(response.cache_fingerprint);
      const entry: SavedCalculate = {
        savedAt: new Date().toISOString(),
        params,
        response,
        cacheFingerprint: fp,
      };
      try {
        localStorage.setItem(storageKey(traderId), JSON.stringify(entry));
      } catch {
        /* ignore quota */
      }
      setSavedParams(params);
    },
    [traderId],
  );

  const loadSaved = useCallback(() => {
    try {
      const raw = localStorage.getItem(storageKey(traderId));
      if (!raw) return;
      const entry = JSON.parse(raw) as SavedCalculate;
      setData(entry.response);
      setSavedParams(entry.params);
      setCapital(entry.params.capital);
      setPeriod(entry.params.period);
      setCustomFrom(entry.params.customFrom);
      setCustomTo(entry.params.customTo);
      setTargetInput(String(entry.params.targetPct));
    } catch {
      /* ignore corrupt */
    }
  }, [traderId]);

  useEffect(() => {
    loadSaved();
  }, [loadSaved]);

  const checkCacheStale = useCallback(async () => {
    if (!savedParams || !data) {
      setMetaChecked(true);
      return;
    }
    const savedFp = str(data.cache_fingerprint);
    if (!savedFp) {
      setMetaChecked(true);
      return;
    }
    try {
      const q = new URLSearchParams();
      if (savedParams.period === "custom") {
        if (savedParams.customFrom) q.set("from", new Date(savedParams.customFrom).toISOString());
        if (savedParams.customTo) q.set("to", new Date(savedParams.customTo).toISOString());
        q.set("period", "custom");
      } else {
        q.set("period", savedParams.period);
      }
      const meta = await client.request<Record<string, unknown>>(
        `/api/v1/source-traders/${traderId}/analytics/calculate-meta?${q.toString()}`,
      );
      setCacheStale(str(meta.cache_fingerprint) !== savedFp);
    } catch {
      setCacheStale(false);
    } finally {
      setMetaChecked(true);
    }
  }, [client, traderId, savedParams, data]);

  useEffect(() => {
    void checkCacheStale();
  }, [checkCacheStale]);

  const run = useCallback(async () => {
    if (calcInFlight.current || loading) return;
    if (!body) {
      setError("Enter valid capital and target coverage (1–100%).");
      return;
    }
    calcInFlight.current = true;
    setLoading(true);
    setError(null);
    try {
      const res = await requestWithTimeout<CalcResponse>(
        client,
        `/api/v1/source-traders/${traderId}/analytics/calculate`,
        { method: "POST", body: JSON.stringify(body) },
        SOURCE_TRADER_CALCULATE_TIMEOUT_MS,
        "Calculation timed out. Try 30D or wait and recalculate.",
      );
      setData(res);
      persist(res, currentParams);
      setCacheStale(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Calculation failed");
    } finally {
      setLoading(false);
      calcInFlight.current = false;
    }
  }, [body, client, traderId, loading, persist, currentParams]);

  const scaling = (data?.scaling ?? {}) as Record<string, unknown>;
  const refK1 = (scaling.reference_at_k_one ?? {}) as Record<string, unknown>;
  const status = str(scaling.status);
  const exclusions = (scaling.exclusions ?? {}) as Record<string, unknown>;
  const atCap = (scaling.at_available_capital ?? {}) as Record<string, unknown>;
  const limits = (scaling.coverage_limits ?? {}) as Record<string, unknown>;
  const minDiag = (scaling.minimum_order_diagnostics ?? {}) as Record<string, unknown>;
  const dq = (data?.data_quality ?? {}) as Record<string, unknown>;
  const breakdown = (data?.event_breakdown ?? scaling.event_breakdown ?? {}) as Record<
    string,
    unknown
  >;
  const warnings = (dq.warnings ?? []) as string[];
  const tradesBackfillComplete =
    dq.trades_backfill_complete === true || dq.backfill_complete === true;
  const activityBackfillComplete = dq.activity_backfill_complete === true;
  const hasResult = data != null && !loading;
  const divisor = num(scaling.capital_divisor);
  const coeff = scaling.recommended_copy_coefficient;
  const maxK = num(atCap.max_copy_coefficient_for_target);
  const appliedK = num(atCap.applied_copy_coefficient);

  return (
    <div style={{ display: "grid", gap: 20 }}>
      <header className="panel" style={{ display: "grid", gap: 12 }}>
        <div>
          <h2 style={{ margin: 0 }}>Copy capital calculator</h2>
          <p className="muted" style={{ margin: "6px 0 0" }}>
            For <strong>{alias}</strong>: how much to scale down copy size to match historical trade
            coverage at your capital (estimate only).
          </p>
        </div>

        {hasResult ? (
          <p className="muted" style={{ margin: 0, fontSize: 14 }}>
            Last calculated: {parseCalcAt(data.calculated_at) || "—"}
            {savedParams ? (
              <>
                {" "}
                · {usd(savedParams.capital)} · {savedParams.period === "custom" ? "Custom" : savedParams.period}{" "}
                · target {savedParams.targetPct}%
              </>
            ) : null}
          </p>
        ) : null}

        {paramsDirty ? (
          <p className="warn" style={{ margin: 0 }}>
            Parameters changed — the result below used your previous settings. Click Recalculate to update.
          </p>
        ) : null}

        {metaChecked && cacheStale ? (
          <p className="warn" style={{ margin: 0 }}>
            Trade history changed since this calculation. Recalculate for an up-to-date result.
          </p>
        ) : null}

        <form
          className="form-stack"
          onSubmit={(e: FormEvent) => {
            e.preventDefault();
            void run();
          }}
          style={{ position: "relative" }}
        >
          {loading ? (
            <div
              className="panel"
              style={{
                position: "absolute",
                inset: 0,
                zIndex: 2,
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                background: "rgba(250, 247, 240, 0.88)",
                border: "1px dashed var(--border, #ccc)",
                margin: 0,
              }}
              role="status"
              aria-live="polite"
            >
              <p style={{ margin: 0, textAlign: "center" }}>
                <strong>Calculating…</strong>
                <br />
                <span className="muted" style={{ fontSize: 14 }}>
                  30D is usually under a minute; 90D can take 1–3 minutes.
                </span>
              </p>
            </div>
          ) : null}
          <fieldset disabled={loading} style={{ border: "none", margin: 0, padding: 0 }}>
            <div className="form-field">
              <label htmlFor="calc-capital">Available capital (USD)</label>
              <input
                id="calc-capital"
                inputMode="decimal"
                value={capital}
                onChange={(e) => setCapital(e.target.value)}
                disabled={loading}
              />
            </div>
            <div className="row" style={{ flexWrap: "wrap", gap: 8 }}>
              {PERIODS.map((p) => (
                <button
                  key={p.id}
                  type="button"
                  className={period === p.id ? "" : "ghost"}
                  onClick={() => setPeriod(p.id)}
                  disabled={loading}
                >
                  {p.label}
                </button>
              ))}
            </div>
            {period === "custom" ? (
              <div className="row" style={{ gap: 12, flexWrap: "wrap" }}>
                <input
                  type="datetime-local"
                  value={customFrom}
                  onChange={(e) => setCustomFrom(e.target.value)}
                />
                <input
                  type="datetime-local"
                  value={customTo}
                  onChange={(e) => setCustomTo(e.target.value)}
                />
              </div>
            ) : null}
            <div className="form-field">
              <label htmlFor="calc-target">Target coverage (%)</label>
              <input
                id="calc-target"
                inputMode="numeric"
                min={1}
                max={100}
                value={targetInput}
                onChange={(e) => setTargetInput(e.target.value)}
                disabled={loading}
              />
              {targetValid == null ? (
                <span className="error" style={{ fontSize: 13 }}>
                  Enter a whole number from 1 to 100.
                </span>
              ) : null}
            </div>
            <button type="submit" disabled={loading || targetValid == null} aria-busy={loading}>
              {loading ? "Calculating…" : hasResult ? "Recalculate" : "Calculate"}
            </button>
          </fieldset>
        </form>
        {error ? <p className="error">{error}</p> : null}
        {hasResult && warnings.length ? (
          <ul className="muted" style={{ margin: 0, paddingLeft: 18, fontSize: 14 }}>
            {warnings.map((w) => (
              <li key={w}>{w}</li>
            ))}
          </ul>
        ) : null}
      </header>

      {hasResult ? (
        <section className="panel" style={{ display: "grid", gap: 16 }}>
          {status === "ok" && divisor != null && divisor > 1 ? (
            <div>
              <p style={{ margin: 0, fontSize: 28, fontWeight: 700, lineHeight: 1.2 }}>
                Reduce copy coefficient by {formatDivisor(divisor)}×
              </p>
              <p className="muted" style={{ margin: "8px 0 0" }}>
                Target applies to copyable-at-1.0× trades (invalid rows excluded; sub-minimum buys
                listed below). Sizing uses 1.0× reference capital need ÷ your available capital.
              </p>
            </div>
          ) : status === "ok" ? (
            <div>
              <p style={{ margin: 0, fontSize: 28, fontWeight: 700, lineHeight: 1.2 }}>
                No reduction needed
              </p>
              <p className="muted" style={{ margin: "8px 0 0" }}>
                Your available capital is enough to copy at 1.0× for this target coverage.
              </p>
            </div>
          ) : (
            <div>
              <p style={{ margin: 0, fontSize: 22, fontWeight: 600, lineHeight: 1.3 }}>
                Target coverage not achievable
              </p>
              <p className="muted" style={{ margin: "8px 0 0" }}>
                {str(scaling.user_message) ||
                  "This target cannot be reached at 1.0× copy with current history and minimum order rules."}
              </p>
              {scaling.max_achievable_trade_coverage != null ? (
                <p className="muted" style={{ margin: "6px 0 0", fontSize: 14 }}>
                  Max copyable-at-1.0× coverage at K ≤ 1:{" "}
                  {covPctFrac(scaling.max_achievable_trade_coverage)}
                  {limits.max_all_trades_at_k_one != null ? (
                    <>
                      {" "}
                      · vs all source trades: {covPctFrac(limits.max_all_trades_at_k_one)}
                    </>
                  ) : null}
                </p>
              ) : null}
            </div>
          )}

          {hasResult ? (
            <dl
              style={{
                margin: 0,
                display: "grid",
                gap: 8,
                gridTemplateColumns: "minmax(10rem, auto) 1fr",
                fontSize: 15,
              }}
            >
              {status === "ok" ? (
                <>
                  <dt className="muted">Recommended copy coefficient</dt>
                  <dd style={{ margin: 0 }}>{formatCoeff(coeff)}</dd>
                </>
              ) : null}
              <dt className="muted" title="Minimum starting cash to execute the eligible trade sequence at K = 1.0 (chronological simulation, not sum of all BUYs).">
                Required capital (K = 1.0)
              </dt>
              <dd style={{ margin: 0 }}>
                {usd(str(scaling.required_capital_usd ?? refK1.required_capital_usd))}
              </dd>
              <dt className="muted" title="Cumulative copied BUY notional at K = 1.0; the same cash can be reused after SELLs.">
                Total BUY volume
              </dt>
              <dd style={{ margin: 0 }}>{usd(str(refK1.total_buy_volume_usd))}</dd>
              <dt className="muted" title="Maximum cost basis simultaneously held in open positions at K = 1.0.">
                Peak deployed capital
              </dt>
              <dd style={{ margin: 0 }}>{usd(str(refK1.peak_deployed_capital_usd))}</dd>
              <dt className="muted" title="Time-weighted average deployed capital between period start and end.">
                Average deployed capital
              </dt>
              <dd style={{ margin: 0 }}>{usd(str(refK1.average_deployed_capital_usd))}</dd>
              <dt className="muted">Available capital</dt>
              <dd style={{ margin: 0 }}>{usd(str(data.available_capital_usd))}</dd>
              <dt className="muted">Target coverage</dt>
              <dd style={{ margin: 0 }}>{covPctFrac(data.target_coverage)} (copyable @ 1.0×)</dd>
              <dt className="muted">Max K for target (chrono.)</dt>
              <dd style={{ margin: 0 }}>{maxK != null ? formatCoeff(maxK) : "—"}</dd>
              <dt className="muted">Applied K (snapshot)</dt>
              <dd style={{ margin: 0 }}>{appliedK != null ? formatCoeff(appliedK) : "—"}</dd>
              <dt className="muted">Trade coverage at your capital</dt>
              <dd style={{ margin: 0 }}>
                All: {covPctFrac(atCap.trade_coverage_all)} · Valid:{" "}
                {covPctFrac(atCap.trade_coverage_valid)} · Copyable:{" "}
                {covPctFrac(atCap.trade_coverage_copyable_at_k_one)}
              </dd>
              <dt className="muted">Source trades</dt>
              <dd style={{ margin: 0 }}>{String(exclusions.source_trades ?? "—")}</dd>
              <dt className="muted">Below minimum order</dt>
              <dd style={{ margin: 0 }}>{String(exclusions.below_minimum_order ?? 0)}</dd>
              <dt className="muted">Invalid / malformed</dt>
              <dd style={{ margin: 0 }}>{String(exclusions.invalid_trades ?? 0)}</dd>
              <dt className="muted">Copyable at 1.0×</dt>
              <dd style={{ margin: 0 }}>{String(exclusions.copyable_at_k_one ?? "—")}</dd>
              <dt className="muted">Event breakdown (simulation)</dt>
              <dd style={{ margin: 0, fontSize: 14 }}>
                BUY {String(breakdown.buy_count ?? 0)} · SELL {String(breakdown.sell_count ?? 0)} ·
                winning REDEEM {String(breakdown.winning_redeems ?? 0)} · losing{" "}
                {String(breakdown.losing_redeems ?? 0)} · unresolved REDEEM{" "}
                {String(breakdown.unresolved_redeems ?? 0)}
                {breakdown.skipped_unmatched_redeems != null &&
                Number(breakdown.skipped_unmatched_redeems) > 0 ? (
                  <> · unmatched REDEEM {String(breakdown.skipped_unmatched_redeems)}</>
                ) : null}
              </dd>
              <dt className="muted">Cached REDEEM rows (period)</dt>
              <dd style={{ margin: 0 }}>
                {String(dq.cashflows_in_period ?? 0)} ingested
                {dq.redeem_winning_in_period != null ? (
                  <>
                    {" "}
                    ({String(dq.redeem_winning_in_period)} win / {String(dq.redeem_losing_in_period ?? 0)}{" "}
                    lose / {String(dq.redeem_unresolved_in_period ?? 0)} unresolved)
                  </>
                ) : null}
              </dd>
              <dt className="muted">History completeness</dt>
              <dd style={{ margin: 0, fontSize: 14 }}>
                Trades backfill: {tradesBackfillComplete ? "complete" : "in progress or partial"}
                {" · "}
                Activity backfill: {activityBackfillComplete ? "complete" : "in progress or partial"}
              </dd>
              {minDiag.min_k_exceeds_one === true ? (
                <>
                  <dt className="muted">Min K for smallest BUY</dt>
                  <dd style={{ margin: 0 }}>
                    {formatCoeff(minDiag.min_k_to_clear_minimum)} (not applied; K capped at 1.0 in
                    recommendations)
                  </dd>
                </>
              ) : null}
            </dl>
          ) : null}

          <p className="muted" style={{ margin: 0, fontSize: 13 }}>
            {str(data.disclaimer)}
          </p>
        </section>
      ) : null}
    </div>
  );
}