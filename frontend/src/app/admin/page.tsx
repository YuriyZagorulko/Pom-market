"use client";

import React, { useState } from "react";

export default function AdminDashboardPage() {
  const [globalStop, setGlobalStop] = useState(false);

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
            SUPER_ADMIN Platform Operations Console
          </h1>
          <p style={{ color: "#94a3b8", margin: 0, fontSize: "13px" }}>
            Platform-wide health, service status, queue telemetry, and user suspension controls.
            Signing secrets are cryptographically isolated and never accessible to administrators.
          </p>
        </div>
        <button
          onClick={() => setGlobalStop((prev) => !prev)}
          style={{
            backgroundColor: globalStop ? "#16a34a" : "#dc2626",
            color: "#ffffff",
            border: "none",
            padding: "12px 20px",
            borderRadius: "6px",
            fontWeight: 700,
            cursor: "pointer",
          }}
        >
          {globalStop ? "Clear Global Emergency Stop" : "TRIGGER GLOBAL EMERGENCY STOP"}
        </button>
      </div>

      {/* Service Health & Aggregates */}
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))",
          gap: "16px",
          marginBottom: "28px",
        }}
      >
        {[
          { label: "Platform Status", value: globalStop ? "EMERGENCY STOP" : "HEALTHY" },
          { label: "Monitored Source Wallets", value: "2,000+" },
          { label: "Active Trading Accounts", value: "50+" },
          { label: "Account-to-Source Mappings", value: "100,000+" },
          { label: "Collector / Router / Workers", value: "ONLINE" },
          { label: "Ambiguous Orders in Flight", value: "0" },
        ].map((card) => (
          <div
            key={card.label}
            style={{
              backgroundColor: "#1e293b",
              padding: "16px",
              borderRadius: "8px",
              border: "1px solid #334155",
            }}
          >
            <div style={{ fontSize: "12px", color: "#94a3b8" }}>{card.label}</div>
            <div style={{ fontSize: "20px", fontWeight: 700, marginTop: "6px" }}>
              {card.value}
            </div>
          </div>
        ))}
      </div>

      {/* Registered Users Table */}
      <div style={{ backgroundColor: "#1e293b", padding: "20px", borderRadius: "8px" }}>
        <h3 style={{ marginTop: 0 }}>Registered Platform Users</h3>
        <table style={{ width: "100%", borderCollapse: "collapse", fontSize: "14px" }}>
          <thead>
            <tr style={{ textAlign: "left", borderBottom: "1px solid #334155" }}>
              <th style={{ padding: "8px" }}>User ID</th>
              <th style={{ padding: "8px" }}>Email</th>
              <th style={{ padding: "8px" }}>Roles</th>
              <th style={{ padding: "8px" }}>2FA (TOTP)</th>
              <th style={{ padding: "8px" }}>Status</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td style={{ padding: "8px" }}>1</td>
              <td style={{ padding: "8px" }}>admin@pom.io</td>
              <td style={{ padding: "8px" }}>SUPER_ADMIN, USER</td>
              <td style={{ padding: "8px" }}>Enabled</td>
              <td style={{ padding: "8px", color: "#4ade80" }}>ACTIVE</td>
            </tr>
            <tr>
              <td style={{ padding: "8px" }}>2</td>
              <td style={{ padding: "8px" }}>alice@example.com</td>
              <td style={{ padding: "8px" }}>USER</td>
              <td style={{ padding: "8px" }}>Enabled</td>
              <td style={{ padding: "8px", color: "#4ade80" }}>ACTIVE</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  );
}
