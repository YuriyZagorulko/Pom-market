"use client";

import { useParams } from "next/navigation";
import { Desk } from "@/components/desk";
import { StrategyDetailView } from "@/components/strategy-analytics/strategy-detail-view";

export default function StrategyPage() {
  const params = useParams<{ id: string }>();
  return (
    <Desk>
      <StrategyDetailView id={params.id} />
    </Desk>
  );
}
