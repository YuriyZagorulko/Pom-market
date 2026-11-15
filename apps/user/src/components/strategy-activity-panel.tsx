"use client";

import { useEffect, useMemo, useState } from "react";
import { FilterChips } from "@/components/filter-chips";
import { PaginationBar } from "@/components/pagination-bar";
import { SimpleFieldLabel } from "@/components/simple-field-label";
import {
  CopyDecisionActivityCard,
  type CopyDecisionActivityRow,
} from "@/components/copy-decision-activity-card";
import { StrategyActivityDatePresets } from "@/components/strategy-activity-date-presets";
import { usePagedList } from "@/lib/use-paged-list";
import { useStrategyActivityQuery } from "@/lib/use-strategy-activity-query";

const TYPE_OPTIONS = [
  { value: "", label: "All types" },
  { value: "buy", label: "Buy" },
  { value: "sell", label: "Sell" },
  { value: "fill", label: "Fill" },
  { value: "settlement", label: "Settlement" },
  { value: "closed", label: "Closed" },
  { value: "capital", label: "Capital" },
];

const STATUS_OPTIONS = [
  { value: "", label: "All statuses" },
  { value: "executed", label: "Executed" },
  { value: "partial", label: "Partial fill" },
  { value: "pending", label: "Pending" },
  { value: "skipped", label: "Skipped" },
  { value: "failed", label: "Failed" },
  { value: "no_order", label: "No order" },
  { value: "settled", label: "Settled (resolution)" },
];

export function StrategyActivityPanel({ strategyId }: { strategyId: string }) {
  const list = useStrategyActivityQuery();
  const path = useMemo(
    () => `/api/v1/strategies/${strategyId}/analytics/activity?${list.apiParams.toString()}`,
    [strategyId, list.apiParams],
  );
  const { items, total, limit, offset, loading, error } = usePagedList<CopyDecisionActivityRow>(path);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [searchDraft, setSearchDraft] = useState(list.read("q"));

  useEffect(() => {
    setSearchDraft(list.read("q"));
  }, [list]);

  useEffect(() => {
    const t = window.setTimeout(() => {
      const trimmed = searchDraft.trim();
      const cur = list.read("q");
      if (trimmed === cur) return;
      list.setParams({ q: trimmed || null }, true);
    }, 350);
    return () => window.clearTimeout(t);
  }, [searchDraft, list]);

  const pages = total === 0 ? 0 : Math.max(1, Math.ceil(total / limit));
  useEffect(() => {
    if (total > 0 && list.page > pages) {
      list.setParams({ page: String(pages) });
    }
  }, [total, list.page, pages, list]);

  return (
    <section className="strategy-activity-panel" style={{ display: "grid", gap: 12 }}>
      <div>
        <h2 style={{ margin: "0 0 4px" }}>Activity</h2>
        <p className="muted" style={{ margin: 0, fontSize: 14 }}>
          Full copy pipeline and PAPER settlement history for this strategy.
        </p>
      </div>
      <StrategyActivityDatePresets
        from={list.read("from")}
        to={list.read("to")}
        onChange={(patch) => list.setParams(patch, true)}
      />
      <div className="filters list-filters">
        <div className="form-field" style={{ flex: "1 1 200px" }}>
          <SimpleFieldLabel htmlFor="act-search">Search</SimpleFieldLabel>
          <input
            id="act-search"
            type="search"
            value={searchDraft}
            placeholder="Market, event, outcome…"
            onChange={(e) => setSearchDraft(e.target.value)}
          />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="act-type">Type</SimpleFieldLabel>
          <select
            id="act-type"
            value={list.read("activity_type")}
            onChange={(e) => list.setParams({ activity_type: e.target.value || null }, true)}
          >
            {TYPE_OPTIONS.map((o) => (
              <option key={o.label} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="act-status">Status</SimpleFieldLabel>
          <select
            id="act-status"
            value={list.read("status")}
            onChange={(e) => list.setParams({ status: e.target.value || null }, true)}
          >
            {STATUS_OPTIONS.map((o) => (
              <option key={o.label} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="act-from">From</SimpleFieldLabel>
          <input
            id="act-from"
            type="date"
            value={list.read("from").slice(0, 10)}
            onChange={(e) => list.setParams({ from: e.target.value || null }, true)}
          />
        </div>
        <div className="form-field">
          <SimpleFieldLabel htmlFor="act-to">To</SimpleFieldLabel>
          <input
            id="act-to"
            type="date"
            value={list.read("to").slice(0, 10)}
            onChange={(e) => list.setParams({ to: e.target.value || null }, true)}
          />
        </div>
      </div>
      <div className="row" style={{ justifyContent: "space-between", alignItems: "center" }}>
        <FilterChips
          chips={list.chips}
          onRemove={(name) => list.setParams({ [name]: null }, true)}
          onClear={() => list.clearAll()}
        />
        <button type="button" className="ghost" onClick={() => list.clearAll()}>
          Clear filters
        </button>
      </div>
      {loading && <p className="muted">Loading activity…</p>}
      {error && <p className="error">{error}</p>}
      {!loading && !error && items.length === 0 && (
        <p className="muted panel activity-empty">
          No activity for these filters.{" "}
          <button type="button" className="ghost" onClick={() => list.clearAll()}>
            Reset filters
          </button>
        </p>
      )}
      {!loading && !error && items.length > 0 && (
        <ul className="activity-feed">
          {items.map((r) => (
            <CopyDecisionActivityCard
              key={r.decision_id}
              row={r}
              open={expanded === r.decision_id}
              onToggle={() => setExpanded((cur) => (cur === r.decision_id ? null : r.decision_id))}
            />
          ))}
        </ul>
      )}
      {!loading && total > 0 && (
        <PaginationBar
          total={total}
          limit={limit}
          offset={offset}
          page={list.page}
          pageSizeOptions={list.pageSizeOptions}
          onPage={(p) => list.setParams({ page: String(p) })}
          onLimit={(n) => list.setParams({ limit: String(n), page: "1" })}
          showFirstLast
        />
      )}
    </section>
  );
}
