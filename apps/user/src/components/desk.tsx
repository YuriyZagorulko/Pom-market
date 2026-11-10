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
  ["/trades", "Wallet activity"],
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

export function useResource<T>(
  path: string,
  options?: { refreshMs?: number },
) {
  const { client, user } = useSession();
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [nonce, setNonce] = useState(0);
  const [refreshMs] = useState(options?.refreshMs);

  useEffect(() => {
    if (!user) {
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);

    const fetchData = async (opts: { initial: boolean }) => {
      try {
        const d = await client.request<T>(path);
        if (cancelled) return;
        setData(d);
        setError("");
        if (opts.initial) setLoading(false);
      } catch (err) {
        if (cancelled) return;
        if (opts.initial) {
          setError(err instanceof Error ? err.message : String(err));
          setLoading(false);
        } else {
          console.warn(`Resource poll failed for ${path}:`, err);
        }
      }
    };

    void fetchData({ initial: true });

    let timer: ReturnType<typeof setTimeout> | null = null;
    let handleVisibility = (() => {}) as () => void;
    if (refreshMs) {
      const schedule = () => {
        if (timer) clearTimeout(timer);
        timer = setTimeout(() => {
          void fetchData({ initial: false });
          schedule();
        }, refreshMs);
      };
      handleVisibility = () => {
        if (document.visibilityState === "hidden") {
          if (timer) {
            clearTimeout(timer);
            timer = null;
          }
        } else {
          schedule();
        }
      };
      schedule();
      document.addEventListener("visibilitychange", handleVisibility);
    }

    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
      document.removeEventListener("visibilitychange", handleVisibility);
    };
  }, [client, path, user, nonce, refreshMs]);

  return {
    data,
    error,
    loading,
    reload: () => setNonce((n) => n + 1),
  };
}

export function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return <label>{label}<div>{children}</div></label>;
}

export async function submitJson(event: FormEvent<HTMLFormElement>, action: (body: Record<string, FormDataEntryValue>) => Promise<void>) {
  event.preventDefault();
  const body = Object.fromEntries(new FormData(event.currentTarget).entries());
  await action(body);
}
