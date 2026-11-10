"use client";

import { useResource } from "@/components/desk";

type TradingAccount = {
  id: string;
  name: string;
};

export function AccountSelect({
  value,
  onChange,
  placeholder = "All accounts",
}: {
  value: string;
  onChange: (accountId: string | null) => void;
  placeholder?: string;
}) {
  const { data, loading, error } = useResource<TradingAccount[]>("/api/v1/accounts");
  const accounts = Array.isArray(data) ? data : [];

  return (
    <label className="account-select">
      <span className="muted" style={{ fontSize: "0.85rem" }}>
        Account
      </span>
      <select
        value={value}
        disabled={loading}
        onChange={(e) => onChange(e.target.value || null)}
        aria-label="Trading account"
      >
        <option value="">{loading ? "Loading…" : placeholder}</option>
        {accounts.map((a) => (
          <option key={a.id} value={a.id}>
            {a.name?.trim() || a.id.slice(0, 8)}
          </option>
        ))}
      </select>
      {error && (
        <span className="error" style={{ fontSize: "0.8rem" }}>
          {error}
        </span>
      )}
      {!loading && !error && !accounts.length && (
        <span className="muted" style={{ fontSize: "0.8rem" }}>
          No accounts yet
        </span>
      )}
    </label>
  );
}
