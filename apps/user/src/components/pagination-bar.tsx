"use client";

import { pageRange } from "@/lib/paged";

export function PaginationBar({
  total,
  limit,
  offset,
  page,
  pageSizeOptions,
  onPage,
  onLimit,
}: {
  total: number;
  limit: number;
  offset: number;
  page: number;
  pageSizeOptions: number[];
  onPage: (page: number) => void;
  onLimit: (limit: number) => void;
}) {
  const pages = total === 0 ? 0 : Math.max(1, Math.ceil(total / limit));
  const canPrev = total > 0 && page > 1;
  const canNext = total > 0 && page < pages;
  return (
    <div className="list-pagination row">
      <span className="muted">{pageRange(offset, limit, total)}</span>
      <div className="row" style={{ gap: 8 }}>
        <select
          value={limit}
          onChange={(e) => onLimit(Number(e.target.value))}
          aria-label="Page size"
        >
          {pageSizeOptions.map((n) => (
            <option key={n} value={n}>
              {n} / page
            </option>
          ))}
        </select>
        <button type="button" className="ghost" disabled={!canPrev} onClick={() => onPage(page - 1)}>
          Previous
        </button>
        <span className="muted">
          Page {page} of {Math.max(pages, 1)}
        </span>
        <button type="button" className="ghost" disabled={!canNext} onClick={() => onPage(page + 1)}>
          Next
        </button>
      </div>
    </div>
  );
}
