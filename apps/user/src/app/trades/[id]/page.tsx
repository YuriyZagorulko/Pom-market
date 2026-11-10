"use client";

import { useParams } from "next/navigation";
import { Desk, useResource } from "@/components/desk";
import { TradeDetailView } from "@/components/trade-detail-view";
import type { SourceTrade } from "@/lib/source-trade-format";

export default function TradePage() {
  const params = useParams<{ id: string }>();
  return (
    <Desk>
      <Detail id={params.id} />
    </Desk>
  );
}

function Detail({ id }: { id: string }) {
  const { data, error, loading } = useResource<{
    trade: SourceTrade;
    links?: Record<string, string | null>;
    decisions?: unknown[];
    orders?: unknown[];
    fills?: unknown[];
    timeline?: Array<{ id: string; kind: string; message: string; at: string }>;
  }>(`/api/v1/trades/${id}`);
  if (loading) return <p>Loading…</p>;
  if (error) return <p className="error">{error}</p>;
  if (!data?.trade) return <p className="error">Trade not found.</p>;
  return <TradeDetailView data={data} />;
}
