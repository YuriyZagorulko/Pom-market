"use client";

import Link from "next/link";
import { Desk, useResource } from "@/components/desk";

export default function AccountsPage() {
  return <Desk><List /></Desk>;
}

function List() {
  const { data, error } = useResource<Array<Record<string, string | null>>>("/api/v1/accounts");
  if (error) return <p className="error">{error}</p>;
  return (
    <section>
      <div className="row"><h1>Polymarket accounts</h1><Link className="button" href="/accounts/new">Add account</Link></div>
      {!data?.length && <p>No accounts yet.</p>}
      <div className="grid">
        {data?.map((account) => (
          <Link key={String(account.id)} href={`/accounts/${account.id}`} className="card">
            <b>{account.name}</b>
            <div className="muted">{account.funder_address}</div>
            <div>{account.operating_mode} · {account.status}</div>
            <div>Balance: {account.last_exchange_balance_usd ?? "unknown"}</div>
          </Link>
        ))}
      </div>
    </section>
  );
}
