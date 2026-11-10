"use client";

import Link from "next/link";
import { useParams, useRouter } from "next/navigation";
import { FormEvent, useMemo, useState } from "react";
import { Desk, useResource } from "@/components/desk";
import { useSession } from "@/lib/session";
import {
  BUDGET_PRESETS,
  COPY_PERCENT_PRESETS,
  percentToProportion,
} from "@/lib/copy-sizing";

type PreviewRow = {
  side: string;
  market_title?: string | null;
  source_notional_usd: string;
  raw_copy_notional_usd?: string | null;
  final_notional_usd?: string | null;
  result: string;
  reason?: string | null;
};

export default function NewStrategyPage() {
  const params = useParams<{ id: string }>();
  return (
    <Desk>
      <Wizard traderId={params.id} />
    </Desk>
  );
}

function Wizard({ traderId }: { traderId: string }) {
  const router = useRouter();
  const { client } = useSession();
  const trader = useResource<Record<string, unknown>>(`/api/v1/source-traders/${traderId}`);
  const accounts = useResource<Array<Record<string, string>>>(`/api/v1/accounts`);

  const [budget, setBudget] = useState("2000");
  const [copyPercent, setCopyPercent] = useState("10");
  const [customBudget, setCustomBudget] = useState(false);
  const [accountId, setAccountId] = useState("");
  const [maxTrade, setMaxTrade] = useState("200");
  const [maxExposure, setMaxExposure] = useState("");
  const [minOrder, setMinOrder] = useState("1");
  const [copyBuy, setCopyBuy] = useState(true);
  const [copySell, setCopySell] = useState(true);
  const [preview, setPreview] = useState<{ rows: PreviewRow[]; disclaimer?: string } | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  const paperAccounts = useMemo(
    () => (accounts.data ?? []).filter((a) => a.operating_mode === "paper_trading"),
    [accounts.data],
  );

  const selectedAccount = (accounts.data ?? []).find((a) => a.id === accountId);
  const monitoring = Boolean(trader.data?.monitoring_enabled);

  async function runPreview() {
    setError("");
    if (!accountId) {
      setError("Choose a trading account.");
      return;
    }
    const proportion = percentToProportion(Number(copyPercent));
    if (proportion == null) {
      setError("Copy percentage must be between 0 and 100.");
      return;
    }
    setBusy(true);
    try {
      const body = {
        trading_account_id: accountId,
        sizing_mode: "proportional",
        copy_percent: copyPercent,
        strategy_budget_usd: budget,
        max_position_usd: maxTrade,
        max_exposure_usd: maxExposure || budget,
        min_order_usd: minOrder,
        copy_buy: copyBuy,
        copy_sell: copySell,
        limit: 15,
      };
      const res = await client.request<{ rows: PreviewRow[]; disclaimer?: string }>(
        `/api/v1/source-traders/${traderId}/copy-preview`,
        { method: "POST", body: JSON.stringify(body) },
      );
      setPreview(res);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Preview failed");
    } finally {
      setBusy(false);
    }
  }

  async function createStrategy(event: FormEvent) {
    event.preventDefault();
    setError("");
    if (!monitoring) {
      setError("Turn on monitoring for this source wallet first.");
      return;
    }
    if (!accountId) {
      setError("Choose a paper trading account.");
      return;
    }
    if (selectedAccount?.operating_mode !== "paper_trading") {
      setError("For now, pick an account in Paper trading mode.");
      return;
    }
    const proportion = percentToProportion(Number(copyPercent));
    if (proportion == null) {
      setError("Invalid copy percentage.");
      return;
    }
  if (
      !confirm(
        `Create copy strategy?\n\nSource: ${trader.data?.alias}\nAccount: ${selectedAccount?.name}\nBudget: $${budget}\nCopy: ${copyPercent}% of each source trade\nMax per trade: $${maxTrade}`,
      )
    ) {
      return;
    }
    setBusy(true);
    try {
      const created = await client.request<{ id: string }>("/api/v1/strategies", {
        method: "POST",
        body: JSON.stringify({
          trading_account_id: accountId,
          source_trader_id: traderId,
          sizing_mode: "proportional",
          copy_percent: copyPercent,
          strategy_budget_usd: budget,
          max_position_usd: maxTrade,
          max_exposure_usd: maxExposure || budget,
          min_order_usd: minOrder,
          copy_buy: copyBuy,
          copy_sell: copySell,
        }),
      });
      router.push(`/strategies/${created.id}`);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Create failed");
    } finally {
      setBusy(false);
    }
  }

  if (trader.error) return <p className="error">{trader.error}</p>;
  if (!trader.data) return <p>Loading…</p>;

  return (
    <section style={{ display: "grid", gap: 20, maxWidth: 720 }}>
      <p><Link href={`/traders/${traderId}`}>← {String(trader.data.alias)}</Link></p>
      <h1>Create copy strategy</h1>

      <div className="panel" style={{ display: "grid", gap: 8 }}>
        <h2 style={{ margin: 0 }}>1. Source account</h2>
        <div className="muted">{String(trader.data.wallet_address)}</div>
        <p style={{ margin: 0 }}>
          Monitoring: <strong>{monitoring ? "Enabled" : "Disabled"}</strong>
        </p>
        {!monitoring && (
          <p className="warn">Enable monitoring on the trader page before creating a strategy.</p>
        )}
      </div>

      <form className="panel" style={{ display: "grid", gap: 10 }} onSubmit={(e) => e.preventDefault()}>
        <h2 style={{ margin: 0 }}>2. Target trading account</h2>
        {paperAccounts.length === 0 ? (
          <p className="warn">
            No paper trading account found.{" "}
            <Link href="/accounts/new">Create an account</Link>, connect credentials, set mode to Paper trading, and set paper cash.
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

        <h2 style={{ margin: "12px 0 0" }}>3. Trading budget</h2>
        <p className="muted" style={{ margin: 0 }}>
          Maximum capital this strategy may hold in open positions plus reserved open orders — not a lifetime BUY cap.
        </p>
        <div className="filters" style={{ flexWrap: "wrap" }}>
          {BUDGET_PRESETS.map((p) => (
            <button
              key={p}
              type="button"
              className={budget === String(p) && !customBudget ? "" : "ghost"}
              onClick={() => { setBudget(String(p)); setCustomBudget(false); }}
            >
              ${p.toLocaleString()}
            </button>
          ))}
          <button type="button" className={customBudget ? "" : "ghost"} onClick={() => setCustomBudget(true)}>
            Custom
          </button>
        </div>
        {customBudget && (
          <input value={budget} onChange={(e) => setBudget(e.target.value)} placeholder="Custom USD" />
        )}
        {!customBudget && <div>Selected: <strong>${Number(budget).toLocaleString()}</strong></div>}

        <h2 style={{ margin: "12px 0 0" }}>4. Copy sizing</h2>
        <p className="muted" style={{ margin: 0 }}>
          Percent of each <strong>source trade notional</strong> (not % of source wallet capital). Example: source BUY $500 at 10% → your ~$50 BUY.
        </p>
        <div className="filters" style={{ flexWrap: "wrap" }}>
          {COPY_PERCENT_PRESETS.map((p) => (
            <button key={p} type="button" className={copyPercent === String(p) ? "" : "ghost"} onClick={() => setCopyPercent(String(p))}>
              {p}%
            </button>
          ))}
        </div>
        <input value={copyPercent} onChange={(e) => setCopyPercent(e.target.value)} placeholder="Custom %" />

        <h2 style={{ margin: "12px 0 0" }}>5. Risk limits</h2>
        <input value={maxTrade} onChange={(e) => setMaxTrade(e.target.value)} placeholder="Max per trade USD" />
        <input value={maxExposure} onChange={(e) => setMaxExposure(e.target.value)} placeholder="Max exposure (default = budget)" />
        <input value={minOrder} onChange={(e) => setMinOrder(e.target.value)} placeholder="Min order USD" />
        <label><input type="checkbox" checked={copyBuy} onChange={(e) => setCopyBuy(e.target.checked)} /> Copy BUY</label>
        <label><input type="checkbox" checked={copySell} onChange={(e) => setCopySell(e.target.checked)} /> Copy SELL</label>

        <div className="filters">
          <button type="button" onClick={runPreview} disabled={busy}>Preview recent trades</button>
          <button type="button" onClick={createStrategy} disabled={busy || !monitoring}>
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
                <th>Copy %</th>
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
