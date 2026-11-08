"use client";

import React, { useEffect, useState } from "react";
import type { DashboardSummary } from "../types/platform";
import { PlatformApiClient } from "../lib/api";

const FALLBACK_SUMMARY: DashboardSummary = {
  total_trading_accounts: 3,
  connected_accounts: 3,
  disconnected_accounts: 0,
  active_copy_trading_accounts: 2,
  monitored_source_traders: 5,
  total_copied_trades: 42,
  daily_pnl_usd: "128.40",
  weekly_pnl_usd: "492.15",
  monthly_pnl_usd: "1840.90",
  current_exposure_usd: "340.00",
  account_performance: [
    {
      account_id: 1,
      display_name: "Alpha Gnosis Safe #1",
      operating_mode: "PAPER_TRADING",
      connection_status: "CONNECTED",
      trading_status: "ACTIVE",
      emergency_stop: false,
      available_cash_usd: "1128.40",
      current_exposure_usd: "220.00",
      total_equity_usd: "1348.40",
      daily_pnl_usd: "84.20",
      weekly_pnl_usd: "310.00",
      monthly_pnl_usd: "1120.50",
    },
    {
      account_id: 2,
      display_name: "Sports Momentum Safe #2",
      operating_mode: "PAPER_TRADING",
      connection_status: "CONNECTED",
      trading_status: "ACTIVE",
      emergency_stop: false,
      available_cash_usd: "890.00",
      current_exposure_usd: "120.00",
      total_equity_usd: "1010.00",
      daily_pnl_usd: "44.20",
      weekly_pnl_usd: "182.15",
      monthly_pnl_usd: "720.40",
    },
    {
      account_id: 3,
      display_name: "Watch-Only Sandbox #3",
      operating_mode: "MONITORING",
      connection_status: "CONNECTED",
      trading_status: "ACTIVE",
      emergency_stop: false,
      available_cash_usd: "1000.00",
      current_exposure_usd: "0.00",
      total_equity_usd: "1000.00",
      daily_pnl_usd: "0.00",
      weekly_pnl_usd: "0.00",
      monthly_pnl_usd: "0.00",
    },
  ],
  recent_orders: [
    {
      id: 101,
      trading_account_id: 1,
      operating_mode: "PAPER_TRADING",
      token_id: "2446159436675684781373961997321275913450",
      side: "BUY",
      limit_price: "0.520000",
      requested_size_shares: "40.000000",
      filled_size_shares: "40.000000",
      status: "FILLED",
      error_message: null,
      order_submitted_ts: new Date().toISOString(),
    },
  ],
  recent_fills: [
    {
      id: 201,
      order_id: 101,
      trading_account_id: 1,
      operating_mode: "PAPER_TRADING",
      token_id: "2446159436675684781373961997321275913450",
      side: "BUY",
      filled_shares: "40.000000",
      avg_fill_price: "0.500000",
      notional_usd: "20.000000",
      fee_usd: "0.200000",
      realized_pnl_usd: "0.000000",
      fill_ts: new Date().toISOString(),
    },
  ],
  execution_errors: [],
};

export default function UserDashboardPage() {
  const [summary, setSummary] = useState<DashboardSummary>(FALLBACK_SUMMARY);

  useEffect(() => {
    const token =
      typeof window !== "undefined" ? localStorage.getItem("pom_access_token") : null;
    if (token) {
      PlatformApiClient.getDashboardSummary(token)
        .then(setSummary)
        .catch(() => {});
    }
  }, []);

  const kpis = [
    { label: "Total Trading Accounts", value: summary.total_trading_accounts },
    {
      label: "Connected / Disconnected",
      value: `${summary.connected_accounts} / ${summary.disconnected_accounts}`,
    },
    { label: "Active Copy Accounts", value: summary.active_copy_trading_accounts },
    { label: "Monitored Source Traders", value: summary.monitored_source_traders },
    { label: "Total Copied Trades", value: summary.total_copied_trades },
    { label: "Current Exposure", value: `$${summary.current_exposure_usd}` },
    { label: "Daily PnL", value: `$${summary.daily_pnl_usd}` },
    { label: "Weekly / Monthly PnL", value: `$${summary.weekly_pnl_usd} / $${summary.monthly_pnl_usd}` },
  ];

  return (
    <div>
      <h1 style={{ fontSize: "24px", marginBottom: "8px" }}>
        Tenant Copy-Trading Dashboard
      </h1>
      <p style={{ color: "#94a3b8", marginBottom: "24px" }}>
        Isolated multi-account portfolio telemetry, PnL, exposure, and real-time order execution.
      </p>

      {/* KPI Grid */}
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fit, minmax(220px, 1fr))",
          gap: "16px",
          marginBottom: "32px",
        }}
      >
        {kpis.map((item) => (
          <div
            key={item.label}
            style={{
              backgroundColor: "#1e293b",
              padding: "16px",
              borderRadius: "8px",
              border: "1px solid #334155",
            }}
          >
            <div style={{ fontSize: "12px", color: "#94a3b8" }}>{item.label}</div>
            <div style={{ fontSize: "22px", fontWeight: 700, marginTop: "6px" }}>
              {item.value}
            </div>
          </div>
        ))}
      </div>

      {/* Account-Level Performance Table */}
      <h2 style={{ fontSize: "18px", marginBottom: "12px" }}>
        Trading Accounts & Performance
      </h2>
      <div
        style={{
          backgroundColor: "#1e293b",
          borderRadius: "8px",
          padding: "16px",
          marginBottom: "32px",
          overflowX: "auto",
        }}
      >
        <table style={{ width: "100%", borderCollapse: "collapse", fontSize: "14px" }}>
          <thead>
            <tr style={{ textAlign: "left", borderBottom: "1px solid #334155" }}>
              <th style={{ padding: "8px" }}>Account</th>
              <th style={{ padding: "8px" }}>Operating Mode</th>
              <th style={{ padding: "8px" }}>Status</th>
              <th style={{ padding: "8px" }}>Available Cash</th>
              <th style={{ padding: "8px" }}>Current Exposure</th>
              <th style={{ padding: "8px" }}>Daily PnL</th>
              <th style={{ padding: "8px" }}>Monthly PnL</th>
            </tr>
          </thead>
          <tbody>
            {summary.account_performance.map((acct) => (
              <tr
                key={acct.account_id}
                style={{ borderBottom: "1px solid #0f172a" }}
              >
                <td style={{ padding: "10px 8px" }}>
                  <a
                    href={`/accounts/${acct.account_id}`}
                    style={{ color: "#38bdf8", textDecoration: "none", fontWeight: 600 }}
                  >
                    {acct.display_name}
                  </a>
                </td>
                <td style={{ padding: "10px 8px" }}>{acct.operating_mode}</td>
                <td style={{ padding: "10px 8px" }}>
                  {acct.emergency_stop ? "EMERGENCY STOPPED" : acct.trading_status}
                </td>
                <td style={{ padding: "10px 8px" }}>${acct.available_cash_usd}</td>
                <td style={{ padding: "10px 8px" }}>${acct.current_exposure_usd}</td>
                <td style={{ padding: "10px 8px", color: "#4ade80" }}>
                  ${acct.daily_pnl_usd}
                </td>
                <td style={{ padding: "10px 8px", color: "#4ade80" }}>
                  ${acct.monthly_pnl_usd}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {/* Recent Orders & Fills */}
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "1fr 1fr",
          gap: "24px",
        }}
      >
        <div
          style={{
            backgroundColor: "#1e293b",
            borderRadius: "8px",
            padding: "16px",
          }}
        >
          <h3 style={{ marginTop: 0 }}>Recent Orders</h3>
          {summary.recent_orders.map((ord) => (
            <div
              key={ord.id}
              style={{
                padding: "10px 0",
                borderBottom: "1px solid #334155",
                fontSize: "13px",
              }}
            >
              <strong>
                #{ord.id} [{ord.operating_mode}] {ord.side}
              </strong>{" "}
              — {ord.filled_size_shares}/{ord.requested_size_shares} shares @ limit $
              {ord.limit_price} ({ord.status})
            </div>
          ))}
        </div>

        <div
          style={{
            backgroundColor: "#1e293b",
            borderRadius: "8px",
            padding: "16px",
          }}
        >
          <h3 style={{ marginTop: 0 }}>Recent Fills & Execution Errors</h3>
          {summary.recent_fills.map((fill) => (
            <div
              key={fill.id}
              style={{
                padding: "10px 0",
                borderBottom: "1px solid #334155",
                fontSize: "13px",
              }}
            >
              <strong>
                Fill #{fill.id} ({fill.side})
              </strong>{" "}
              — {fill.filled_shares} shares @ ${fill.avg_fill_price} (Fee: $
              {fill.fee_usd}, PnL: ${fill.realized_pnl_usd})
            </div>
          ))}
          {summary.execution_errors.map((err) => (
            <div
              key={err.job_id}
              style={{
                padding: "8px 0",
                color: "#f87171",
                fontSize: "13px",
              }}
            >
              Job #{err.job_id} [{err.status}]: {err.reason}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
