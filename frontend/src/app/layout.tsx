import React from "react";

export const metadata = {
  title: "Pom-Market | Multi-Tenant Polymarket Copy-Trading Platform",
  description:
    "Self-hosted multi-tenant Polymarket copy-trading platform with source-attributed virtual positions and independent account execution.",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body
        style={{
          margin: 0,
          fontFamily: "system-ui, -apple-system, sans-serif",
          backgroundColor: "#0b0f19",
          color: "#e2e8f0",
        }}
      >
        <header
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
            padding: "16px 32px",
            borderBottom: "1px solid #1e293b",
            backgroundColor: "#0f172a",
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: "16px" }}>
            <strong style={{ fontSize: "18px", color: "#38bdf8" }}>
              Pom-Market Copy-Trading
            </strong>
            <span
              style={{
                fontSize: "12px",
                padding: "2px 8px",
                borderRadius: "4px",
                backgroundColor: "#1e293b",
                color: "#94a3b8",
              }}
            >
              Multi-Tenant v1.0
            </span>
          </div>
          <nav style={{ display: "flex", gap: "20px", fontSize: "14px" }}>
            <a href="/" style={{ color: "#e2e8f0", textDecoration: "none" }}>
              User Dashboard
            </a>
            <a
              href="/accounts/1"
              style={{ color: "#e2e8f0", textDecoration: "none" }}
            >
              Trading Accounts
            </a>
            <a
              href="/traders/0x06dc51826bc524d9a83770e7de9dd7e005b04524"
              style={{ color: "#e2e8f0", textDecoration: "none" }}
            >
              Source Traders
            </a>
            <a
              href="/admin"
              style={{ color: "#f59e0b", textDecoration: "none" }}
            >
              Admin Console
            </a>
          </nav>
        </header>
        <main style={{ maxWidth: "1360px", margin: "0 auto", padding: "24px 32px" }}>
          {children}
        </main>
      </body>
    </html>
  );
}
