/**
 * Display formatting for numeric HTML inputs (not currency display).
 * Preserves significant digits; strips trailing fractional zeros only.
 */

const DECIMAL_RE = /^(-?)(\d*)(?:\.(\d*))?$/;

export function formatNumericInputDisplay(
  value: string | number | null | undefined,
): string {
  if (value === null || value === undefined) return "";
  const raw = typeof value === "number" ? String(value) : String(value).trim();
  if (raw === "") return "";

  const m = raw.match(DECIMAL_RE);
  if (!m) {
    return raw;
  }
  const [, sign, intPart, fracPart] = m;
  const intNormalized = intPart === "" ? "0" : intPart;
  if (fracPart === undefined) {
    return `${sign}${intNormalized}`;
  }
  const fracTrimmed = fracPart.replace(/0+$/, "");
  if (!fracTrimmed) {
    return `${sign}${intNormalized}`;
  }
  return `${sign}${intNormalized}.${fracTrimmed}`;
}

/** True when the string is a valid in-progress decimal edit (e.g. "10.", "-0.0"). */
export function isIntermediateNumericInput(value: string): boolean {
  const t = value.trim();
  if (t === "" || t === "-" || t === "." || t === "-.") return true;
  return /^-?\d*\.?\d*$/.test(t);
}
