"use client";

import Link from "next/link";
import { Desk, useResource } from "@/components/desk";

export default function RiskPage() {
  return <Desk><Risk /></Desk>;
}

function Risk() {
  const { data } = useResource<Array<Record<string, string>>>("/api/v1/accounts");
  return (
    <section>
      <h1>Risk management</h1>
      <p className="muted">Limits are stored per account. Open an account to edit the emergency stop, order size, daily loss, exposure, and paper cash.</p>
      <table>
        <thead><tr><th>Account</th><th>Mode</th><th>Emergency stop</th><th></th></tr></thead>
        <tbody>
          {data?.map((account) => (
            <tr key={account.id}>
              <td>{account.name}</td>
              <td>{account.operating_mode}</td>
              <td>{String(account.emergency_stop)}</td>
              <td><Link href={`/accounts/${account.id}`}>Edit</Link></td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
