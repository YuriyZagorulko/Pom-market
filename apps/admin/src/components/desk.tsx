"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect } from "react";
import { useSession } from "@/lib/session";

const links = [["/", "Overview"], ["/users", "Users"], ["/health", "Health"], ["/events", "Events"], ["/settings", "Settings"]];

export function Desk({ children }: { children: React.ReactNode }) {
  const { user, ready, logout } = useSession();
  const pathname = usePathname();
  const router = useRouter();
  useEffect(() => { if (ready && !user) router.replace("/login"); }, [ready, user, router]);
  if (!ready || !user) return <main>Loading session…</main>;
  return (
    <div className="shell">
      <nav>
        <strong>ADMIN CONSOLE</strong>
        {links.map(([href, label]) => <Link key={href} href={href}>{pathname === href ? `• ${label}` : label}</Link>)}
        <button className="ghost" onClick={() => logout().then(() => router.push("/login"))}>Log out</button>
      </nav>
      <main>{children}</main>
    </div>
  );
}
