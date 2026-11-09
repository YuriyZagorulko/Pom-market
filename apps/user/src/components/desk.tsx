"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { FormEvent, useEffect, useState } from "react";
import { useSession } from "@/lib/session";

const links = [
  ["/", "Dashboard"],
  ["/accounts", "Accounts"],
  ["/traders", "Source traders"],
  ["/strategies", "Strategies"],
  ["/activity", "Activity"],
  ["/trades", "Source trades"],
  ["/decisions", "Decisions"],
  ["/orders", "Orders"],
  ["/executions", "Executions"],
  ["/positions", "Positions"],
  ["/risk", "Risk"],
  ["/settings", "Settings"],
];

export function Desk({ children }: { children: React.ReactNode }) {
  const { user, ready, logout } = useSession();
  const pathname = usePathname();
  const router = useRouter();
  useEffect(() => {
    if (ready && !user) router.replace("/login");
  }, [ready, user, router]);
  if (!ready || !user) return <main>Loading session…</main>;
  return (
    <div className="shell">
      <nav>
        <strong>Pom-Market</strong>
        {links.map(([href, label]) => (
          <Link key={href} href={href} className={pathname === href ? "active" : ""}>{label}</Link>
        ))}
        <button className="ghost" onClick={() => logout().then(() => router.push("/login"))}>Log out</button>
      </nav>
      <main>{children}</main>
    </div>
  );
}

export function useResource<T>(path: string) {
  const { client, user } = useSession();
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState("");
  const [nonce, setNonce] = useState(0);
  useEffect(() => {
    if (!user) return;
    client.request<T>(path).then(setData).catch((err) => setError(err.message));
  }, [client, path, user, nonce]);
  return { data, error, reload: () => setNonce((n) => n + 1) };
}

export function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return <label>{label}<div>{children}</div></label>;
}

export async function submitJson(event: FormEvent<HTMLFormElement>, action: (body: Record<string, FormDataEntryValue>) => Promise<void>) {
  event.preventDefault();
  const body = Object.fromEntries(new FormData(event.currentTarget).entries());
  await action(body);
}
