"use client";

import { Suspense, type ReactNode } from "react";
import { Desk } from "@/components/desk";
import { AccountSelect } from "@/components/account-select";
import { DateRangePresets } from "@/components/date-range-presets";
import { FilterChips } from "@/components/filter-chips";
import { PaginationBar } from "@/components/pagination-bar";
import { SimpleFieldLabel } from "@/components/simple-field-label";
import { useToast } from "@/components/toast";
import { useListQuery } from "@/lib/use-list-query";
import { usePagedList } from "@/lib/use-paged-list";

export type FilterField = {
  type: "text" | "select" | "date" | "account";
  name: string;
  label?: string;
  placeholder?: string;
  options?: Array<{ value: string; label: string }>;
};

function hasDateFields(fields: FilterField[]): boolean {
  return fields.some((f) => f.type === "date" && (f.name === "from" || f.name === "to"));
}

function PagedListBody({
  title,
  subtitle,
  apiPath,
  fields,
  render,
}: {
  title: string;
  subtitle?: string;
  apiPath: string;
  fields: FilterField[];
  render: (ctx: { items: Record<string, unknown>[]; loading: boolean; error: string }) => ReactNode;
}) {
  const toast = useToast();
  const list = useListQuery();
  const path = `${apiPath}?${list.apiParams.toString()}`;
  const { items, total, limit, offset, loading, error, reload } = usePagedList<Record<string, unknown>>(path);

  return (
    <section className="paged-list">
      <div className="row">
        <div>
          <h1>{title}</h1>
          {subtitle && (
            <p className="muted" style={{ margin: 0 }}>
              {subtitle}
            </p>
          )}
        </div>
        <button
          type="button"
          className="ghost"
          onClick={() => {
            reload();
            toast.info("List refreshed");
          }}
        >
          Refresh
        </button>
      </div>
      {hasDateFields(fields) && (
        <DateRangePresets
          from={list.read("from")}
          to={list.read("to")}
          onChange={(patch) => list.setParams(patch, true)}
        />
      )}
      <div className="filters list-filters">
        {fields.map((field) => {
          const title = field.label ?? field.placeholder ?? field.name;
          const id = `filter-${field.name}`;
          return (
            <div key={field.name} className="form-field">
              <SimpleFieldLabel htmlFor={field.type === "account" ? undefined : id}>{title}</SimpleFieldLabel>
              {field.type === "account" ? (
                <AccountSelect
                  value={list.read(field.name)}
                  onChange={(accountId) => list.setParams({ account_id: accountId }, true)}
                />
              ) : field.type === "select" ? (
                <select
                  id={id}
                  value={list.read(field.name)}
                  onChange={(e) => list.setParams({ [field.name]: e.target.value || null }, true)}
                >
                  {field.options?.map((opt) => (
                    <option key={opt.value} value={opt.value}>
                      {opt.label}
                    </option>
                  ))}
                </select>
              ) : (
                <input
                  id={id}
                  type={field.type === "date" ? "date" : "text"}
                  defaultValue={list.read(field.name)}
                  onBlur={(e) =>
                    list.setParams(
                      { [field.name]: (field.type === "date" ? e.target.value : e.target.value.trim()) || null },
                      true,
                    )
                  }
                />
              )}
            </div>
          );
        })}
      </div>
      <FilterChips
        chips={list.chips}
        onRemove={(name) => list.setParams({ [name]: null }, true)}
        onClear={() => list.clearAll()}
      />
      {loading && <p className="muted">Loading…</p>}
      {error && <p className="error">{error}</p>}
      {!loading && !error && !items.length && (
        <p className="muted panel activity-empty">No results for the current filters.</p>
      )}
      {render({ items, loading, error })}
      {!loading && (
        <PaginationBar
          total={total}
          limit={limit}
          offset={offset}
          page={list.page}
          pageSizeOptions={list.pageSizeOptions}
          onPage={(p) => list.setParams({ page: String(p) })}
          onLimit={(n) => list.setParams({ limit: String(n), page: "1" })}
        />
      )}
    </section>
  );
}

export function PagedListPage({
  title,
  subtitle,
  apiPath,
  fields,
  render,
}: {
  title: string;
  subtitle?: string;
  apiPath: string;
  fields: FilterField[];
  render: (ctx: { items: Record<string, unknown>[]; loading: boolean; error: string }) => ReactNode;
}) {
  return (
    <Desk>
      <Suspense fallback={<p>Loading…</p>}>
        <PagedListBody title={title} subtitle={subtitle} apiPath={apiPath} fields={fields} render={render} />
      </Suspense>
    </Desk>
  );
}
