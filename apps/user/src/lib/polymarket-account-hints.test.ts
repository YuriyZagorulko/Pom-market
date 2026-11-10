import { describe, expect, it } from "vitest";
import { funderRequiredForSignature } from "./polymarket-account-hints";

describe("funderRequiredForSignature", () => {
  it("requires funder for proxy, safe, and 1271", () => {
    expect(funderRequiredForSignature("eoa")).toBe(false);
    expect(funderRequiredForSignature("poly_proxy")).toBe(true);
    expect(funderRequiredForSignature("poly_gnosis_safe")).toBe(true);
    expect(funderRequiredForSignature("poly_1271")).toBe(true);
  });
});
