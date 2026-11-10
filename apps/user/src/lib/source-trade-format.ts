export type SourceTrade = {
  id: string;
  source_wallet: string;
  tx_hash?: string | null;
  token_id: string;
  condition_id?: string | null;
  outcome?: string | null;
  market_title?: string | null;
  market_slug?: string | null;
  event_title?: string | null;
  event_slug?: string | null;
  market_icon?: string | null;
  activity_type?: string | null;
  usdc_size?: string | number | null;
  side: string;
  price: string | number;
  size_shares: string | number;
  notional_usd: string | number;
  event_ts: string;
  origin?: string;
  status?: string;
};

export function shortenAddress(addr: string, head = 6, tail = 4): string {
  const v = addr.trim();
  if (v.length <= head + tail + 3) return v;
  return `${v.slice(0, head)}…${v.slice(-tail)}`;
}

export function shortenHash(hash: string, head = 10, tail = 6): string {
  const v = hash.trim();
  if (v.length <= head + tail + 3) return v;
  return `${v.slice(0, head)}…${v.slice(-tail)}`;
}

export function formatShares(value: string | number | null | undefined): string | null {
  if (value === null || value === undefined || value === "") return null;
  const n = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(n)) return null;
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: 4 }).format(n);
}

export function formatUsdc(value: string | number | null | undefined): string | null {
  if (value === null || value === undefined || value === "") return null;
  const n = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(n)) return null;
  return new Intl.NumberFormat("en-US", { style: "currency", currency: "USD", maximumFractionDigits: 2 }).format(n);
}

export function formatPricePerShare(value: string | number | null | undefined): string | null {
  if (value === null || value === undefined || value === "") return null;
  const n = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(n)) return null;
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 3,
  }).format(n);
}

export function polymarketUrl(trade: Pick<SourceTrade, "event_slug" | "market_slug">): string | null {
  const event = (trade.event_slug || "").trim();
  const market = (trade.market_slug || "").trim();
  if (market && (!event || event !== market)) {
    return `https://polymarket.com/market/${market}`;
  }
  if (event) return `https://polymarket.com/event/${event}`;
  if (market) return `https://polymarket.com/market/${market}`;
  return null;
}

export function polygonscanUrl(txHash?: string | null): string | null {
  const tx = (txHash || "").trim();
  if (!tx || !/^0x[a-fA-F0-9]+$/.test(tx)) return null;
  return `https://polygonscan.com/tx/${tx}`;
}

export function activityLabel(type?: string | null): string {
  const t = (type || "trade").toLowerCase();
  if (t === "trade") return "Trade";
  return t.charAt(0).toUpperCase() + t.slice(1);
}

export function formatEventTime(iso: string): string {
  try {
    return new Intl.DateTimeFormat(undefined, {
      month: "short",
      day: "numeric",
      year: "numeric",
      hour: "numeric",
      minute: "2-digit",
    }).format(new Date(iso));
  } catch {
    return iso;
  }
}

export function displayNotional(trade: SourceTrade): string | null {
  if (trade.usdc_size !== null && trade.usdc_size !== undefined && trade.usdc_size !== "") {
    return formatUsdc(trade.usdc_size);
  }
  return formatUsdc(trade.notional_usd);
}
