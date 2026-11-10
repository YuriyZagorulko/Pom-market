import { describe, expect, it } from "vitest";
import {
  buildApiParams,
  buildFilterChips,
  clearAllParams,
  mergeListParams,
  normalizeLimit,
  normalizePage,
} from "./list-query-core";

describe("normalizePage", () => {
  it("defaults missing to 1", () => expect(normalizePage(undefined)).toBe(1));
  it("rejects invalid", () => {
    expect(normalizePage("x")).toBe(1);
    expect(normalizePage("-2")).toBe(1);
    expect(normalizePage("0")).toBe(1);
  });
  it("floors positive values", () => expect(normalizePage("2.9")).toBe(2));
});

describe("normalizeLimit", () => {
  it("defaults to 25", () => expect(normalizeLimit(undefined)).toBe(25));
  it("allows 50 and 100", () => {
    expect(normalizeLimit("50")).toBe(50);
    expect(normalizeLimit("100")).toBe(100);
  });
  it("rejects out of range", () => {
    expect(normalizeLimit("10")).toBe(25);
    expect(normalizeLimit("500")).toBe(25);
  });
});

describe("mergeListParams", () => {
  it("sets and removes params", () => {
    const base = new URLSearchParams("side=buy&page=3");
    const next = mergeListParams(base, "", { side: null, q: "btc" }, false);
    expect(next.get("side")).toBeNull();
    expect(next.get("q")).toBe("btc");
    expect(next.get("page")).toBe("3");
  });

  it("resets page when resetPage true", () => {
    const base = new URLSearchParams("page=4&side=sell");
    const next = mergeListParams(base, "", { side: "buy" }, true);
    expect(next.get("page")).toBeNull();
    expect(next.get("side")).toBe("buy");
  });

  it("clears empty filter values", () => {
    const base = new URLSearchParams("status=filled");
    const next = mergeListParams(base, "", { status: "" }, true);
    expect(next.get("status")).toBeNull();
  });
});

describe("buildApiParams", () => {
  it("forwards filters and page/limit", () => {
    const sp = new URLSearchParams("side=buy&from=2025-01-01&q=foo&page=2&limit=50");
    const api = buildApiParams(sp, "", 2, 50);
    expect(api.get("page")).toBe("2");
    expect(api.get("limit")).toBe("50");
    expect(api.get("side")).toBe("buy");
    expect(api.get("from")).toBe("2025-01-01");
    expect(api.get("q")).toBe("foo");
    expect(api.get("page")).toBe("2");
  });
});

describe("buildFilterChips", () => {
  it("omits empty params", () => {
    const sp = new URLSearchParams("side=buy");
    expect(buildFilterChips(sp, "")).toEqual([{ name: "side", label: "Side", value: "buy" }]);
  });
});

describe("clearAllParams", () => {
  it("clears everything", () => {
    const sp = new URLSearchParams("side=buy&page=2");
    expect(clearAllParams(sp, "").toString()).toBe("");
  });
});
