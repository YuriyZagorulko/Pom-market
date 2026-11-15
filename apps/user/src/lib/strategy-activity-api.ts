import { readSearchParam } from "@/lib/list-query-core";
import { apiDateBounds } from "@/lib/strategy-activity-dates";

const ACTIVITY_FORWARD = ["q", "activity_type", "status", "from", "to"] as const;

export function buildStrategyActivityApiParams(
  searchParams: URLSearchParams,
  prefix: string,
  page: number,
  limit: number,
): URLSearchParams {
  const params = new URLSearchParams();
  params.set("limit", String(limit));
  params.set("page", String(page));
  for (const name of ACTIVITY_FORWARD) {
    const v = readSearchParam(searchParams, prefix, name);
    if (!v) continue;
    if (name === "from" || name === "to") continue;
    params.set(name === "activity_type" ? "activity_type" : name, v);
  }
  const from = readSearchParam(searchParams, prefix, "from");
  const to = readSearchParam(searchParams, prefix, "to");
  const bounds = apiDateBounds(from, to);
  if (bounds.from) params.set("from", bounds.from);
  if (bounds.to) params.set("to", bounds.to);
  return params;
}
