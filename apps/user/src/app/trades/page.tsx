"use client";

import { Suspense } from "react";
import { Desk } from "@/components/desk";
import { WalletActivityFeed } from "@/components/source-trade-feed";

export default function Page() {
  return (
    <Desk>
      <Suspense fallback={<p>Loading…</p>}>
        <WalletActivityFeed />
      </Suspense>
    </Desk>
  );
}
