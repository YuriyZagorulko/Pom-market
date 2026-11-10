/** Verified external links — do not add query params or credential material. */
export const POLYMARKET_ACCOUNT_LINKS = {
  magicKeyExport: "https://reveal.magic.link/polymarket",
  walletsAuthDocs: "https://docs.polymarket.com/trading/wallets-auth",
  profileSettings: "https://polymarket.com/settings",
} as const;

export type SignatureTypeValue =
  | "eoa"
  | "poly_proxy"
  | "poly_gnosis_safe"
  | "poly_1271";

export function funderRequiredForSignature(signatureType: SignatureTypeValue): boolean {
  return signatureType !== "eoa";
}
