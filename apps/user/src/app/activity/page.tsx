"use client";

import { PagedListPage } from "@/components/paged-list-page";

export default function Page() {
  return (
    <PagedListPage
      title="Activity"
      subtitle="Account-level timeline (copy pipeline events)"
      apiPath="/api/v1/activity"
      fields={[
        { type: "text", name: "q", placeholder: "Search message" },
        { type: "text", name: "kind", placeholder: "Event type" },
        { type: "date", name: "from" },
        { type: "date", name: "to" },
      ]}
      render={({ items }) => (
        <div className="timeline panel">
          {items.map((row) => (
            <div key={String(row.id)} className="timeline-row">
              <span className="pill">{String(row.kind)}</span>
              <strong>{String(row.at)}</strong>
              <p style={{ margin: "4px 0 0" }}>{String(row.message)}</p>
            </div>
          ))}
        </div>
      )}
    />
  );
}
