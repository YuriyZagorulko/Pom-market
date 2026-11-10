"use client";

import { useMemo, useState } from "react";
import { shortenAddress } from "@/lib/source-trade-format";

type Trader = { id: string; alias: string; wallet_address: string };

export function WalletMultiselect({
  traders,
  selected,
  onChange,
}: {
  traders: Trader[];
  selected: string[];
  onChange: (ids: string[]) => void;
}) {
  const [filter, setFilter] = useState("");
  const [open, setOpen] = useState(false);
  const filtered = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return traders;
    return traders.filter(
      (t) => t.alias.toLowerCase().includes(q) || t.wallet_address.toLowerCase().includes(q),
    );
  }, [traders, filter]);
  const label =
    selected.length === 0 || selected.length === traders.length
      ? "All wallets"
      : `${selected.length} wallet${selected.length === 1 ? "" : "s"}`;
  return (
    <div className="wallet-multiselect">
      <button type="button" className="ghost wallet-multiselect-trigger" onClick={() => setOpen((v) => !v)}>
        {label}
      </button>
      {open && (
        <div className="wallet-multiselect-panel panel">
          <input
            placeholder="Search alias or address"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
          <div className="wallet-multiselect-actions row">
            <button type="button" className="ghost" onClick={() => onChange(traders.map((t) => t.id))}>
              Select all
            </button>
            <button type="button" className="ghost" onClick={() => onChange([])}>
              Clear
            </button>
          </div>
          <ul className="wallet-multiselect-list">
            {filtered.map((t) => (
              <li key={t.id}>
                <label>
                  <input
                    type="checkbox"
                    checked={selected.length === 0 || selected.includes(t.id)}
                    onChange={(e) => {
                      if (selected.length === 0) {
                        onChange(traders.map((x) => x.id).filter((id) => id !== t.id || e.target.checked));
                        return;
                      }
                      if (e.target.checked) onChange([...new Set([...selected, t.id])]);
                      else onChange(selected.filter((id) => id !== t.id));
                    }}
                  />
                  <span>{t.alias}</span>
                  <span className="muted">{shortenAddress(t.wallet_address)}</span>
                </label>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
