"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { FormEvent, useMemo, useState } from "react";
import { useResource } from "@/components/desk";
import {
  buildCopyStrategyCreateBody,
  validateCopyStrategyCreateInput,
  type AllocationMode,
  type SizingMode,
} from "@/lib/copy-strategy-create";
import { SimpleFieldLabel } from "@/components/simple-field-label";
import { useToast } from "@/components/toast";
import { BUDGET_PRESETS, COPY_PERCENT_PRESETS } from "@/lib/copy-sizing";
import { useSession } from "@/lib/session";

type PreviewRow = {
  side: string;
  market_title?: string | null;
  source_notional_usd: string;
  raw_copy_notional_usd?: string | null;
  final_notional_usd?: string | null;
  result: string;
  reason?: string | null;
};

type Props = {
  /** When set (trader detail flow), hide trader picker and lock source wallet. */
  fixedTraderId?: string;
  backHref?: string;
  backLabel?: string;
};

export function CopyStrategyCreateWizard({ fixedTraderId, backHref, backLabel }: Props) {
  const router = useRouter();
  const { client } = useSession();
  const toast = useToast();
  const traders = useResource<Array<Record<string, unknown>>>("/api/v1/source-traders", {
    enabled: !fixedTraderId,
  });
  const traderOne = useResource<Record<string, unknown>>(`/api/v1/source-traders/${fixedTraderId ?? "_"}`, {
    enabled: Boolean(fixedTraderId),
  });
  const accounts = useResource<Array<Record<string, string>>>(`/api/v1/accounts`);

  const [traderId, setTraderId] = useState(fixedTraderId ?? "");
  const [accountId, setAccountId] = useState("");
  const [allocationMode, setAllocationMode] = useState<AllocationMode>("fixed_usd");
  const [allocationPercent, setAllocationPercent] = useState("25");
  const [sizingMode, setSizingMode] = useState<SizingMode>("portfolio_ratio");
  const [budget, setBudget] = useState("2000");
  const [customBudget, setCustomBudget] = useState(false);
  const [copyPercent, setCopyPercent] = useState("10");
  const [fixedAmountUsd, setFixedAmountUsd] = useState("25");
  const [multiplier, setMultiplier] = useState("1");
  const [maxTrade, setMaxTrade] = useState("200");
  const [maxExposure, setMaxExposure] = useState("");
  const [maxOpenPositions, setMaxOpenPositions] = useState("10");
  const [maxSlippageBps, setMaxSlippageBps] = useState("150");
  const [minOrder, setMinOrder] = useState("1");
  const [cooldownSeconds, setCooldownSeconds] = useState("0");
  const [copyBuy, setCopyBuy] = useState(true);
  const [copySell, setCopySell] = useState(true);
  const [preview, setPreview] = useState<{ rows: PreviewRow[]; disclaimer?: string } | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  const effectiveTraderId = fixedTraderId ?? traderId;
  const allocationSummary = useResource<{
    account_equity_usd: string;
    unallocated_usd: string;
    strategies: Array<{ alias: string; effective_cap_usd: string; allocation_mode: string }>;
  }>(`/api/v1/accounts/${accountId || "_"}/allocation-summary`, { enabled: Boolean(accountId) });
  const traderList = traders.data ?? [];
  const selectedTrader =
    fixedTraderId && traderOne.data
      ? traderOne.data
      : traderList.find((t) => String(t.id) === effectiveTraderId);
  const paperAccounts = useMemo(
    () => (accounts.data ?? []).filter((a) => a.operating_mode === "paper_trading"),
    [accounts.data],
  );
  const selectedAccount = (accounts.data ?? []).find((a) => a.id === accountId);
  const monitoring = Boolean(selectedTrader?.monitoring_enabled);
  const sourceEquity = useResource<{
    equity_usd?: string | null;
    status: string;
    quality_note?: string | null;
    age_seconds?: number;
    observed_at?: string;
  }>(`/api/v1/source-traders/${effectiveTraderId || "_"}/equity`, {
    enabled: Boolean(effectiveTraderId) && monitoring,
  });

  const validationInput = {
    tradingAccountId: accountId,
    sourceTraderId: effectiveTraderId,
    allocationMode,
    allocationPercent,
    sizingMode,
    strategyBudgetUsd: budget,
    maxPositionUsd: maxTrade,
    maxExposureUsd: maxExposure,
    maxOpenPositions,
    maxSlippageBps,
    minOrderUsd: minOrder,
    cooldownSeconds,
    copyBuy,
    copySell,
    copyPercent,
    fixedAmountUsd,
    multiplier,
    monitoringEnabled: monitoring,
    accountOperatingMode: selectedAccount?.operating_mode,
  };

  async function runPreview() {
    setError("");
    const validation = validateCopyStrategyCreateInput(validationInput);
    if (validation) {
      setError(validation);
      toast.error(validation);
      return;
    }
    setBusy(true);
    try {
      const body = {
        ...buildCopyStrategyCreateBody(validationInput),
        limit: 15,
      };
      const res = await client.request<{ rows: PreviewRow[]; disclaimer?: string }>(
        `/api/v1/source-traders/${effectiveTraderId}/copy-preview`,
        { method: "POST", body: JSON.stringify(body) },
      );
      setPreview(res);
      toast.success("Preview loaded");
    } catch (e) {
      const msg = e instanceof Error ? e.message : "Preview failed";
      setError(msg);
      toast.error(msg);
    } finally {
      setBusy(false);
    }
  }

  async function createStrategy(event: FormEvent) {
    event.preventDefault();
    setError("");
    if (!fixedTraderId && !traderId) {
      const msg = "Choose a source trader.";
      setError(msg);
      toast.error(msg);
      return;
    }
    if (!accountId) {
      const msg = "Choose a paper trading account.";
      setError(msg);
      toast.error(msg);
      return;
    }
    if (!monitoring) {
      const msg = "Turn on monitoring for the source wallet before creating a strategy.";
      setError(msg);
      toast.error(msg);
      return;
    }
    if (paperAccounts.length === 0) {
      const msg = "Create a paper trading account first, then try again.";
      setError(msg);
      toast.error(msg);
      return;
    }
    const validation = validateCopyStrategyCreateInput(validationInput);
    if (validation) {
      setError(validation);
      toast.error(validation);
      return;
    }
    const traderLabel = String(selectedTrader?.alias ?? selectedTrader?.wallet_address ?? "trader");
    if (
      !confirm(
        `Create disabled copy strategy?\n\nSource: ${traderLabel}\nAccount: ${selectedAccount?.name}\nBudget: $${budget}\n\nCopying does not start until you enable the strategy.`,
      )
    ) {
      return;
    }
    setBusy(true);
    try {
      const created = await client.request<{ id: string; enabled: boolean }>("/api/v1/strategies", {
        method: "POST",
        body: JSON.stringify(buildCopyStrategyCreateBody(validationInput)),
      });
      if (created.enabled) {
        setError("Server returned an enabled strategy; please disable before use.");
        return;
      }
      toast.success("Strategy created (disabled)");
      router.push(`/strategies/${created.id}`);
    } catch (e) {
      const msg = e instanceof Error ? e.message : "Create failed";
      setError(msg);
      toast.error(msg);
    } finally {
      setBusy(false);
    }
  }

  if (traderOne.error) return <p className="error">{traderOne.error}</p>;
  if (fixedTraderId && !traderOne.data) return <p>Loading…</p>;
  if (!fixedTraderId && traders.error) return <p className="error">{traders.error}</p>;

  const noTraders = !fixedTraderId && (traderList.length === 0);

  return (
    <section style={{ display: "grid", gap: 20, maxWidth: 720 }}>
      {backHref && (
        <p>
          <Link href={backHref}>← {backLabel ?? "Back"}</Link>
        </p>
      )}
      <h1>Create copy strategy</h1>
      <p className="muted" style={{ margin: 0 }}>
        Saving creates a <strong>disabled</strong> strategy. Enable it on the strategy page when you are ready — only
        new source trades after enable are copied.
      </p>

      <form className="panel" style={{ display: "grid", gap: 10 }} onSubmit={createStrategy} noValidate>
        <h2 style={{ margin: 0 }}>Source trader</h2>
        {fixedTraderId && selectedTrader ? (
          <>
            <div className="muted">{String(selectedTrader.wallet_address)}</div>
            <p style={{ margin: 0 }}>
              Monitoring: <strong>{monitoring ? "Enabled" : "Disabled"}</strong>
            </p>
          </>
        ) : noTraders ? (
          <p className="warn">
            No source traders yet. <Link href="/traders/new">Add a source wallet</Link> and turn on monitoring first.
          </p>
        ) : (
          <select required value={traderId} onChange={(e) => setTraderId(e.target.value)}>
            <option value="">Select source trader…</option>
            {traderList.map((t) => (
              <option key={String(t.id)} value={String(t.id)}>
                {String(t.alias)} · {String(t.wallet_address)}
                {t.monitoring_enabled ? "" : " (monitoring off)"}
              </option>
            ))}
          </select>
        )}
        {selectedTrader && !monitoring && (
          <p className="warn">
            Enable monitoring on the{" "}
            <Link href={`/traders/${effectiveTraderId}`}>trader page</Link> before creating a strategy.
          </p>
        )}

        <h2 style={{ margin: "12px 0 0" }}>Trading account</h2>
        {paperAccounts.length === 0 ? (
          <p className="warn">
            No paper trading account found.{" "}
            <Link href="/accounts/new">Create an account</Link>, connect credentials, set mode to Paper trading, and set
            paper cash.
          </p>
        ) : (
          <select required value={accountId} onChange={(e) => setAccountId(e.target.value)}>
            <option value="">Select account…</option>
            {paperAccounts.map((a) => (
              <option key={a.id} value={a.id}>
                {a.name} · {a.operating_mode}
              </option>
            ))}
          </select>
        )}
        {selectedAccount && (
          <p className="muted" style={{ margin: 0 }}>
            Mode: {selectedAccount.operating_mode}. Live copy is not enabled in this release.
          </p>
        )}

        <h2 style={{ margin: "12px 0 0" }}>Capital allocation</h2>
        <p className="muted" style={{ margin: 0 }}>
          Capital reserved for this strategy on the paper account (distinct from per-trade copy sizing).
        </p>
        <select value={allocationMode} onChange={(e) => setAllocationMode(e.target.value as AllocationMode)}>
          <option value="fixed_usd">Fixed USD amount</option>
          <option value="percent_of_account">Percent of account equity</option>
        </select>
        {allocationMode === "fixed_usd" ? (
          <>
            <div className="filters" style={{ flexWrap: "wrap" }}>
              {BUDGET_PRESETS.map((p) => (
                <button
                  key={p}
                  type="button"
                  className={budget === String(p) && !customBudget ? "" : "ghost"}
                  onClick={() => {
                    setBudget(String(p));
                    setCustomBudget(false);
                  }}
                >
                  ${p.toLocaleString()}
                </button>
              ))}
              <button type="button" className={customBudget ? "" : "ghost"} onClick={() => setCustomBudget(true)}>
                Custom
              </button>
            </div>
            {(customBudget || !(BUDGET_PRESETS as readonly number[]).includes(Number(budget))) && (
              <input value={budget} onChange={(e) => setBudget(e.target.value)} placeholder="Allocated USD" />
            )}
          </>
        ) : (
          <input
            value={allocationPercent}
            onChange={(e) => setAllocationPercent(e.target.value)}
            placeholder="Percent of account equity (e.g. 25)"
          />
        )}
        {allocationSummary.data && (
          <p className="muted" style={{ margin: 0 }}>
            Account equity: ${allocationSummary.data.account_equity_usd} · Unallocated: $
            {allocationSummary.data.unallocated_usd}
            {allocationSummary.data.strategies.length > 0 && (
              <> · Other strategies: {allocationSummary.data.strategies.map((s) => s.alias).join(", ")}</>
            )}
          </p>
        )}

        {monitoring && sourceEquity.data && (
          <p className="muted" style={{ margin: 0 }}>
            Source equity ({sourceEquity.data.status}
            {sourceEquity.data.status === "unavailable" ? "" : ", estimated"}):{" "}
            {sourceEquity.data.equity_usd != null ? `$${sourceEquity.data.equity_usd}` : "unavailable"}
            {sourceEquity.data.age_seconds != null ? ` · ${sourceEquity.data.age_seconds}s old` : ""}
            {sourceEquity.data.quality_note ? ` — ${sourceEquity.data.quality_note}` : ""}
          </p>
        )}

        <h2 style={{ margin: "12px 0 0" }}>Copy sizing</h2>
        <select value={sizingMode} onChange={(e) => setSizingMode(e.target.value as SizingMode)}>
          <option value="portfolio_ratio">Portfolio-relative (recommended)</option>
          <option value="proportional">Proportional — % of each source trade</option>
          <option value="fixed_amount">Fixed amount — same USD per copied trade</option>
          <option value="multiplier">Multiplier — scale source notional</option>
        </select>
        {sizingMode === "portfolio_ratio" && (
          <p className="muted" style={{ margin: 0 }}>
            Target order ≈ source notional × (your allocation ÷ source equity). Requires fresh partial+ source equity;
            SELL exits are unchanged.
          </p>
        )}

        {sizingMode === "proportional" && (
          <>
            <p className="muted" style={{ margin: 0 }}>
              Percent of each <strong>source trade notional</strong>, not % of wallet capital.
            </p>
            <div className="filters" style={{ flexWrap: "wrap" }}>
              {COPY_PERCENT_PRESETS.map((p) => (
                <button
                  key={p}
                  type="button"
                  className={copyPercent === String(p) ? "" : "ghost"}
                  onClick={() => setCopyPercent(String(p))}
                >
                  {p}%
                </button>
              ))}
            </div>
            <input value={copyPercent} onChange={(e) => setCopyPercent(e.target.value)} placeholder="Custom %" />
          </>
        )}
        {sizingMode === "fixed_amount" && (
          <input
            value={fixedAmountUsd}
            onChange={(e) => setFixedAmountUsd(e.target.value)}
            placeholder="Fixed amount USD per trade"
          />
        )}
        {sizingMode === "multiplier" && (
          <input value={multiplier} onChange={(e) => setMultiplier(e.target.value)} placeholder="Multiplier (e.g. 1.5)" />
        )}

        <h2 style={{ margin: "12px 0 0" }}>Risk limits</h2>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="wiz-max-position">Max position / per trade (USD)</SimpleFieldLabel>
          <input id="wiz-max-position" value={maxTrade} onChange={(e) => setMaxTrade(e.target.value)} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="wiz-max-exposure" optional>
            Max exposure (USD)
          </SimpleFieldLabel>
          <input id="wiz-max-exposure" value={maxExposure} onChange={(e) => setMaxExposure(e.target.value)} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="wiz-max-positions">Max open positions</SimpleFieldLabel>
          <input id="wiz-max-positions" value={maxOpenPositions} onChange={(e) => setMaxOpenPositions(e.target.value)} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="wiz-slippage">Max slippage (basis points)</SimpleFieldLabel>
          <input id="wiz-slippage" value={maxSlippageBps} onChange={(e) => setMaxSlippageBps(e.target.value)} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="wiz-min-order">Min order (USD)</SimpleFieldLabel>
          <input id="wiz-min-order" value={minOrder} onChange={(e) => setMinOrder(e.target.value)} />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="wiz-cooldown">Cooldown (seconds)</SimpleFieldLabel>
          <input id="wiz-cooldown" value={cooldownSeconds} onChange={(e) => setCooldownSeconds(e.target.value)} />
        </div>
        <label>
          <input type="checkbox" checked={copyBuy} onChange={(e) => setCopyBuy(e.target.checked)} /> Copy BUY
        </label>
        <label>
          <input type="checkbox" checked={copySell} onChange={(e) => setCopySell(e.target.checked)} /> Copy SELL
        </label>

        <div className="filters">
          <button type="button" onClick={runPreview} disabled={busy}>
            Preview recent trades
          </button>
          <button type="submit" disabled={busy}>
            Create disabled strategy
          </button>
        </div>
        {error && <p className="error">{error}</p>}
      </form>

      {preview && (
        <section className="panel">
          <h2>Trade preview (estimate)</h2>
          <p className="muted">{preview.disclaimer}</p>
          <table>
            <thead>
              <tr>
                <th>Side</th>
                <th>Market</th>
                <th>Source</th>
                <th>Copy</th>
                <th>After caps</th>
                <th>Result</th>
              </tr>
            </thead>
            <tbody>
              {preview.rows.map((row, i) => (
                <tr key={i}>
                  <td>{row.side.toUpperCase()}</td>
                  <td>{row.market_title ?? "—"}</td>
                  <td>${row.source_notional_usd}</td>
                  <td>${row.raw_copy_notional_usd ?? "—"}</td>
                  <td>${row.final_notional_usd ?? "—"}</td>
                  <td>
                    {row.result}
                    {row.reason ? <span className="muted"> · {row.reason}</span> : null}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      )}
    </section>
  );
}
