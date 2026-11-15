"use client";

import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useCallback, useMemo } from "react";
import {
  buildFilterChips,
  clearAllParams,
  mergeListParams,
  normalizeLimit,
  normalizePage,
  PAGE_SIZE_OPTIONS,
  readSearchParam,
} from "@/lib/list-query-core";
import { buildStrategyActivityApiParams } from "@/lib/strategy-activity-api";

const CHIP_NAMES = ["q", "activity_type", "status", "from", "to"] as const;

export function useStrategyActivityQuery() {
  const prefix = "act";
  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();

  const read = useCallback(
    (name: string, fallback = "") => readSearchParam(searchParams, prefix, name, fallback),
    [searchParams],
  );

  const page = normalizePage(read("page", "1"));
  const limit = normalizeLimit(read("limit", "25"));

  const setParams = useCallback(
    (patch: Record<string, string | null>, resetPage = false) => {
      const params = mergeListParams(searchParams, prefix, patch, resetPage);
      const q = params.toString();
      router.replace(q ? `${pathname}?${q}` : pathname, { scroll: false });
    },
    [router, pathname, searchParams],
  );

  const clearAll = useCallback(() => {
    const params = clearAllParams(searchParams, prefix, []);
    const q = params.toString();
    router.replace(q ? `${pathname}?${q}` : pathname, { scroll: false });
  }, [router, pathname, searchParams]);

  const apiParams = useMemo(
    () => buildStrategyActivityApiParams(searchParams, prefix, page, limit),
    [searchParams, page, limit],
  );

  const chips = useMemo(() => {
    const all = buildFilterChips(searchParams, prefix);
    return all.filter((c) => (CHIP_NAMES as readonly string[]).includes(c.name));
  }, [searchParams]);

  return {
    read,
    setParams,
    clearAll,
    page,
    limit,
    apiParams,
    chips,
    pageSizeOptions: [...PAGE_SIZE_OPTIONS],
  };
}
