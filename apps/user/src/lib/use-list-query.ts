"use client";

import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useCallback, useMemo } from "react";
import {
  buildApiParams,
  buildFilterChips,
  clearAllParams,
  mergeListParams,
  normalizeLimit,
  normalizePage,
  PAGE_SIZE_OPTIONS,
  readSearchParam,
} from "@/lib/list-query-core";

export type ListQueryState = Record<string, string>;

export function useListQuery(prefix = "") {
  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();

  const read = useCallback(
    (name: string, fallback = "") => readSearchParam(searchParams, prefix, name, fallback),
    [searchParams, prefix],
  );

  const page = normalizePage(read("page", "1"));
  const limit = normalizeLimit(read("limit", "25"));

  const setParams = useCallback(
    (patch: Record<string, string | null>, resetPage = false) => {
      const params = mergeListParams(searchParams, prefix, patch, resetPage);
      const q = params.toString();
      router.replace(q ? `${pathname}?${q}` : pathname, { scroll: false });
    },
    [router, pathname, searchParams, prefix],
  );

  const clearAll = useCallback(
    (keep: string[] = []) => {
      const params = clearAllParams(searchParams, prefix, keep);
      const q = params.toString();
      router.replace(q ? `${pathname}?${q}` : pathname, { scroll: false });
    },
    [router, pathname, searchParams, prefix],
  );

  const apiParams = useMemo(
    () => buildApiParams(searchParams, prefix, page, limit),
    [searchParams, prefix, page, limit],
  );

  const chips = useMemo(() => buildFilterChips(searchParams, prefix), [searchParams, prefix]);

  return { read, setParams, clearAll, page, limit, apiParams, chips, pageSizeOptions: [...PAGE_SIZE_OPTIONS] };
}
