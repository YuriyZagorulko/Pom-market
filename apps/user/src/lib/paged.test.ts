import { describe, expect, it } from "vitest";
import { pageRange, parsePaged } from "./paged";

describe("parsePaged", () => {
  it("parses paged object", () => {
    const body = { items: [{ id: "1" }], total: 10, limit: 25, offset: 0 };
    const p = parsePaged<{ id: string }>(body);
    expect(p.items).toHaveLength(1);
    expect(p.total).toBe(10);
  });

  it("falls back to array", () => {
    const p = parsePaged<string>(["a", "b"]);
    expect(p.items).toEqual(["a", "b"]);
    expect(p.total).toBe(2);
  });

  it("handles garbage safely", () => {
    const p = parsePaged(null);
    expect(p.items).toEqual([]);
    expect(p.total).toBe(0);
  });
});

describe("pageRange", () => {
  it("empty total", () => expect(pageRange(0, 25, 0)).toBe("No results"));
  it("first page", () => expect(pageRange(0, 25, 100)).toMatch(/1–25/));
  it("last partial page", () => expect(pageRange(75, 25, 80)).toMatch(/76–80/));
});
