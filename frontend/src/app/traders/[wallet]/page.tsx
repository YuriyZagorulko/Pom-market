"use client";

import React from "react";

export default function SourceTraderPage({
  params,
}: {
  params: { wallet: string };
}) {
  return (
    <div>
      <h1 style={{ fontSize: "24px", marginBottom: "6px" }}>
        Source Trader Profile — Whale Alpha
      </h1>
      <p style={{ color: "#94a3b8", marginBottom: "24px" }}>
        Wallet Address: <code>{params.wallet}</code> | Observed Trades:{" "}
        <strong>148</strong> | Volume: <strong>$84,210.00</strong>
      </p>

      <div
        style={{
          display: "grid",
          gridTemplateColumns: "1fr 1fr",
          gap: "24px",
        }}
      >
        <div style={{ backgroundColor: "#1e293b", padding: "20px", borderRadius: "8px" }}>
          <h3 style={{ marginTop: 0 }}>
            Your Copying Destination Accounts (Tenant-Scoped)
          </h3>
          <table style={{ width: "100%", borderCollapse: "collapse", fontSize: "14px" }}>
            <thead>
              <tr style={{ textAlign: "left", borderBottom: "1px solid #334155" }}>
                <th style={{ padding: "8px" }}>Account</th>
                <th style={{ padding: "8px" }}>Mode</th>
                <th style={{ padding: "8px" }}>Sizing</th>
                <th style={{ padding: "8px" }}>Max Slippage</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td style={{ padding: "8px" }}>Alpha Gnosis Safe #1</td>
                <td style={{ padding: "8px" }}>PAPER_TRADING</td>
                <td style={{ padding: "8px" }}>FIXED_AMOUNT ($15.00)</td>
                <td style={{ padding: "8px" }}>5.0%</td>
              </tr>
              <tr>
                <td style={{ padding: "8px" }}>Watch-Only Sandbox #3</td>
                <td style={{ padding: "8px" }}>MONITORING</td>
                <td style={{ padding: "8px" }}>FIXED_PERCENTAGE (10%)</td>
                <td style={{ padding: "8px" }}>5.0%</td>
              </tr>
            </tbody>
          </table>
        </div>

        <div style={{ backgroundColor: "#1e293b", padding: "20px", borderRadius: "8px" }}>
          <h3 style={{ marginTop: 0 }}>Recent Public Trades & Active Markets</h3>
          <div style={{ fontSize: "13px", lineHeight: 1.8 }}>
            <div>
              <strong>BUY 120.50 shares @ $0.42</strong> — Lakers vs Celtics (Token{" "}
              <code>2446...7268</code>)
            </div>
            <div>
              <strong>SELL 50.00 shares @ $0.60</strong> — Election Winner (Token{" "}
              <code>1000...0001</code>)
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
