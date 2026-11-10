import { describe, expect, it } from "vitest";
import {
  activityLabel,
  displayNotional,
  formatShares,
  formatUsdc,
  polygonscanUrl,
  polymarketUrl,
  shortenAddress,
} from "./source-trade-format";

describe("source-trade-format", () => {
  it("builds polymarket links from market vs event slugs", () => {
    expect(polymarketUrl({ event_slug: "evt", market_slug: "mkt" })).toBe(
      "https://polymarket.com/market/mkt",
    );
    expect(polymarketUrl({ event_slug: null, market_slug: "only-slug" })).toBe(
      "https://polymarket.com/market/only-slug",
    );
    expect(polymarketUrl({ event_slug: "same", market_slug: "same" })).toBe(
      "https://polymarket.com/event/same",
    );
    expect(polymarketUrl({ event_slug: "", market_slug: "" })).toBeNull();
  });

  it("builds polygonscan links only for valid hashes", () => {
    expect(polygonscanUrl("0xabc123")).toBe("https://polygonscan.com/tx/0xabc123");
    expect(polygonscanUrl("not-a-hash")).toBeNull();
    expect(polygonscanUrl(null)).toBeNull();
  });

  it("formats shares and USDC without inventing zeros", () => {
    expect(formatShares(7)).toBe("7");
    expect(formatShares(null)).toBeNull();
    expect(formatUsdc("12.5")).toBe("$12.50");
    expect(formatUsdc(undefined)).toBeNull();
  });

  it("prefers usdc_size over notional when present", () => {
    expect(
      displayNotional({
        id: "1",
        source_wallet: "0x",
        token_id: "1",
        side: "buy",
        price: 0.5,
        size_shares: 10,
        notional_usd: 5,
        usdc_size: 12.34,
        event_ts: "",
      }),
    ).toBe("$12.34");
    expect(
      displayNotional({
        id: "2",
        source_wallet: "0x",
        token_id: "1",
        side: "sell",
        price: 0.5,
        size_shares: 10,
        notional_usd: 5,
        event_ts: "",
      }),
    ).toBe("$5.00");
  });

  it("labels activity types", () => {
    expect(activityLabel("trade")).toBe("Trade");
    expect(activityLabel("REDEEM")).toBe("Redeem");
  });

  it("shortens wallet addresses", () => {
    expect(shortenAddress("0x1234567890abcdef1234567890abcdef12345678")).toBe("0x1234…5678");
  });
});
