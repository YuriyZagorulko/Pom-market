"use client";

import { Desk } from "@/components/desk";
import { CopyStrategyCreateWizard } from "@/components/copy-strategy-create-wizard";

export default function NewCopyStrategyPage() {
  return (
    <Desk>
      <CopyStrategyCreateWizard backHref="/strategies" backLabel="Strategies" />
    </Desk>
  );
}
