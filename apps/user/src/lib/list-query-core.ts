/** API list filters use UTC calendar dates (`YYYY-MM-DD`), matching `backend/src/api/page.rs`. */

export const PAGE_SIZE_OPTIONS = [25, 50, 100] as const;

const FORWARD_PARAMS = [
  "q",
  "side",
  "status",
  "account_id",
  "wallets",
  "from",
  "to",
  "min_amount",
  "max_amount",
  "min_price",
  "max_price",
  "outcome",
  "kind",
  "activity_type",
  "mode",
] as const;

export type ForwardParam = (typeof FORWARD_PARAMS)[number];

export function queryKey(prefix: string, name: string): string {
  return prefix ? `${prefix}_${name}` : name;
}

export function normalizePage(raw: string | undefined): number {
  const n = Number(raw ?? "1");
  if (!Number.isFinite(n) || n < 1) return 1;
  return Math.floor(n);
}

export function normalizeLimit(raw: string | undefined): number {
  const n = Number(raw ?? "25");
  if (!PAGE_SIZE_OPTIONS.includes(n as (typeof PAGE_SIZE_OPTIONS)[number])) return 25;
  return n;
}

export function readSearchParam(searchParams: URLSearchParams, prefix: string, name: string, fallback = ""): string {
  return searchParams.get(queryKey(prefix, name)) ?? fallback;
}

export function searchParamsToState(searchParams: URLSearchParams, prefix: string): Record<string, string> {
  const out: Record<string, string> = {};
  searchParams.forEach((value, param) => {
    if (prefix) {
      if (!param.startsWith(`${prefix}_`)) return;
      out[param.slice(prefix.length + 1)] = value;
    } else {
      out[param] = value;
    }
  });
  return out;
}

export function mergeListParams(
  searchParams: URLSearchParams,
  prefix: string,
  patch: Record<string, string | null>,
  resetPage: boolean,
): URLSearchParams {
  const params = new URLSearchParams(searchParams.toString());
  for (const [name, value] of Object.entries(patch)) {
    const p = queryKey(prefix, name);
    if (value === null || value === "") params.delete(p);
    else params.set(p, value);
  }
  if (resetPage) params.delete(queryKey(prefix, "page"));
  return params;
}

export function buildApiParams(
  searchParams: URLSearchParams,
  prefix: string,
  page: number,
  limit: number,
): URLSearchParams {
  const params = new URLSearchParams();
  params.set("limit", String(limit));
  params.set("page", String(page));
  for (const name of FORWARD_PARAMS) {
    const v = readSearchParam(searchParams, prefix, name);
    if (v) params.set(name, v);
  }
  return params;
}

export type FilterChip = { name: string; label: string; value: string };

const CHIP_LABELS: Partial<Record<ForwardParam, string>> = {
  side: "Side",
  status: "Status",
  q: "Search",
  wallets: "Wallets",
  outcome: "Outcome",
  from: "From",
  to: "To",
  activity_type: "Type",
  kind: "Kind",
  account_id: "Account",
};

export function buildFilterChips(
  searchParams: URLSearchParams,
  prefix: string,
  accountNames?: Record<string, string>,
): FilterChip[] {
  const labels: FilterChip[] = [];
  for (const name of FORWARD_PARAMS) {
    const v = readSearchParam(searchParams, prefix, name);
    if (!v) continue;
    const label = CHIP_LABELS[name] ?? name;
    const display = name === "account_id" && accountNames?.[v] ? `${accountNames[v]}` : v;
    labels.push({ name, label, value: display });
  }
  return labels;
}

export function clearAllParams(
  searchParams: URLSearchParams,
  prefix: string,
  keep: string[] = [],
): URLSearchParams {
  const params = new URLSearchParams();
  for (const name of keep) {
    const v = searchParams.get(queryKey(prefix, name));
    if (v) params.set(queryKey(prefix, name), v);
  }
  return params;
}
