"use client";

import React, { useState } from "react";
import type { OperatingMode } from "../../../types/platform";

export default function TradingAccountDetailPage({
  params,
}: {
  params: { id: string };
}) {
  const [mode, setMode] = useState<OperatingMode>("MONITORING");
  const [emergencyStop, setEmergencyStop] = useState(false);
  const [confirmPhrase, setConfirmPhrase] = useState("");
  const [statusMessage, setStatusMessage] = useState<string | null>(null);

  const handleModeSwitch = (targetMode: OperatingMode) => {
    if (targetMode === "LIVE_TRADING" && confirmPhrase !== "CONFIRM_LIVE_TRADING") {
      setStatusMessage(
        "Refused: Switching to LIVE_TRADING requires typing CONFIRM_LIVE_TRADING"
      );
      return;
    }
    setMode(targetMode);
    setStatusMessage(`Operating mode updated to ${targetMode}`);
  };

  return (
    <div>
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
          marginBottom: "24px",
        }}
      >
        <div>
          <h1 style={{ fontSize: "24px", marginBottom: "4px" }}>
            Trading Account #{params.id} — Alpha Gnosis Safe
          </h1>
          <div style={{ fontSize: "13px", color: "#94a3b8" }}>
            Signature Type: <strong>POLY_GNOSIS_SAFE</strong> | Funder Proxy:{" "}
            <code>0x1111...1111</code> | Signer EOA: <code>0x90f7...b906</code>
          </div>
        </div>
        <button
          onClick={() => setEmergencyStop((prev) => !prev)}
          style={{
            backgroundColor: emergencyStop ? "#16a34a" : "#dc2626",
            color: "#ffffff",
            border: "none",
            padding: "12px 20px",
            borderRadius: "6px",
            fontWeight: 700,
            cursor: "pointer",
          }}
        >
          {emergencyStop ? "Resume Account Trading" : "EMERGENCY STOP (Kill Switch)"}
        </button>
      </div>

      {statusMessage && (
        <div
          style={{
            padding: "12px 16px",
            backgroundColor: "#1e293b",
            borderLeft: "4px solid #38bdf8",
            marginBottom: "20px",
          }}
        >
          {statusMessage}
        </div>
      )}

      {/* Operating Mode Control Panel */}
      <div
        style={{
          backgroundColor: "#1e293b",
          padding: "20px",
          borderRadius: "8px",
          marginBottom: "24px",
        }}
      >
        <h3 style={{ marginTop: 0 }}>
          Operating Mode Control (Current: {mode})
        </h3>
        <div style={{ display: "flex", gap: "12px", alignItems: "center", flexWrap: "wrap" }}>
          <button
            onClick={() => handleModeSwitch("MONITORING")}
            style={{ padding: "8px 16px", cursor: "pointer" }}
          >
            Set MONITORING (Default)
          </button>
          <button
            onClick={() => handleModeSwitch("PAPER_TRADING")}
            style={{ padding: "8px 16px", cursor: "pointer" }}
          >
            Set PAPER_TRADING
          </button>
          <input
            type="text"
            placeholder="Type CONFIRM_LIVE_TRADING"
            value={confirmPhrase}
            onChange={(e) => setConfirmPhrase(e.target.value)}
            style={{
              padding: "8px 12px",
              borderRadius: "4px",
              border: "1px solid #475569",
              backgroundColor: "#0f172a",
              color: "#e2e8f0",
            }}
          />
          <button
            onClick={() => handleModeSwitch("LIVE_TRADING")}
            style={{
              padding: "8px 16px",
              backgroundColor: "#f59e0b",
              color: "#0f172a",
              fontWeight: 700,
              border: "none",
              borderRadius: "4px",
              cursor: "pointer",
            }}
          >
            Activate LIVE_TRADING
          </button>
        </div>
      </div>

      {/* Balances, Copy Config & Risk Limits */}
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "1fr 1fr",
          gap: "24px",
          marginBottom: "24px",
        }}
      >
        <div style={{ backgroundColor: "#1e293b", padding: "20px", borderRadius: "8px" }}>
          <h3 style={{ marginTop: 0 }}>Copy Strategy Configuration</h3>
          <ul style={{ lineHeight: 1.8, fontSize: "14px" }}>
            <li>Execution Mode: <strong>FIXED_AMOUNT ($20.00)</strong></li>
            <li>Copy Multiplier: <strong>1.0x</strong></li>
            <li>Min / Max Order Size: <strong>$1.00 / $100.00</strong></li>
            <li>BUY / SELL Copying Enabled: <strong>TRUE / TRUE</strong></li>
            <li>Slice Collapse Window: <strong>300s</strong></li>
          </ul>
        </div>
        <div style={{ backgroundColor: "#1e293b", padding: "20px", borderRadius: "8px" }}>
          <h3 style={{ marginTop: 0 }}>Account Risk Limits</h3>
          <ul style={{ lineHeight: 1.8, fontSize: "14px" }}>
            <li>Max Market Exposure: <strong>$500.00</strong></li>
            <li>Max Exposure per Source Trader: <strong>$500.00</strong></li>
            <li>Max Daily Realized Loss: <strong>$200.00</strong></li>
            <li>Max Slippage Tolerance: <strong>5.0%</strong></li>
            <li>Rate Limit: <strong>10 executions / 60s</strong></li>
          </ul>
        </div>
      </div>

      {/* Virtual Positions Attributed by Source Wallet */}
      <div style={{ backgroundColor: "#1e293b", padding: "20px", borderRadius: "8px" }}>
        <h3 style={{ marginTop: 0 }}>
          Source-Attributed Virtual Positions{" "}
          <span style={{ fontSize: "12px", color: "#94a3b8", fontWeight: 400 }}>
            Keyed by (account_id, source_wallet, token_id, operating_mode)
          </span>
        </h3>
        <table style={{ width: "100%", borderCollapse: "collapse", fontSize: "14px" }}>
          <thead>
            <tr style={{ textAlign: "left", borderBottom: "1px solid #334155" }}>
              <th style={{ padding: "8px" }}>Source Trader Wallet</th>
              <th style={{ padding: "8px" }}>Token ID</th>
              <th style={{ padding: "8px" }}>Open Shares</th>
              <th style={{ padding: "8px" }}>Avg Entry</th>
              <th style={{ padding: "8px" }}>Cost Basis</th>
              <th style={{ padding: "8px" }}>Realized PnL</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td style={{ padding: "8px" }}>
                <code>0xaaaa...0001</code>
              </td>
              <td style={{ padding: "8px" }}>
                <code>1000...0001</code>
              </td>
              <td style={{ padding: "8px" }}>20.000000</td>
              <td style={{ padding: "8px" }}>$0.500000</td>
              <td style={{ padding: "8px" }}>$10.000000</td>
              <td style={{ padding: "8px", color: "#4ade80" }}>+$1.840000</td>
            </tr>
            <tr>
              <td style={{ padding: "8px" }}>
                <code>0xbbbb...0002</code>
              </td>
              <td style={{ padding: "8px" }}>
                <code>1000...0001</code>
              </td>
              <td style={{ padding: "8px" }}>80.000000</td>
              <td style={{ padding: "8px" }}>$0.500000</td>
              <td style={{ padding: "8px" }}>$40.000000</td>
              <td style={{ padding: "8px" }}>$0.000000</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  );
}
