"use client";

import { useMemo } from "react";
import { useResource } from "@/components/desk";
import { parsePaged, type Paged } from "@/lib/paged";

export function usePagedList<T>(path: string) {
  const { data, loading, error, reload } = useResource<Paged<T> | T[]>(path);
  const parsed = useMemo(() => parsePaged<T>(data), [data]);
  return { ...parsed, loading, error, reload };
}
