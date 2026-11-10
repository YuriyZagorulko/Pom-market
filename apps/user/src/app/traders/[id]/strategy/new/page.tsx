"use client";

import { useParams } from "next/navigation";
import { Desk } from "@/components/desk";
import { CopyStrategyCreateWizard } from "@/components/copy-strategy-create-wizard";

export default function NewStrategyPage() {
  const params = useParams<{ id: string }>();
  return (
    <Desk>
      <CopyStrategyCreateWizard
        fixedTraderId={params.id}
        backHref={`/traders/${params.id}`}
        backLabel="Source trader"
      />
    </Desk>
  );
}
