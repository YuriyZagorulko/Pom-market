"use client";

import { ApiClient, ApiError, ApiUser } from "@pom/api-client";
import { createContext, useContext, useEffect, useMemo, useState } from "react";

type Session = {
  user: ApiUser | null;
  ready: boolean;
  client: ApiClient;
  login: (email: string, password: string) => Promise<void>;
  logout: () => Promise<void>;
};

const Ctx = createContext<Session | null>(null);

export function SessionProvider({ children, role }: { children: React.ReactNode; role: "user" | "admin" }) {
  const client = useMemo(() => new ApiClient(), []);
  const [user, setUser] = useState<ApiUser | null>(null);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    client.refresh().then((result) => {
      if (result.user.role === role) {
        client.setToken(result.access_token);
        setUser(result.user);
      }
    }).catch(() => undefined).finally(() => setReady(true));
  }, [client, role]);

  const value: Session = {
    user,
    ready,
    client,
    async login(email, password) {
      const result = await client.login(email, password);
      if (result.user.role !== role) {
        await client.logout();
        throw new ApiError(403, role === "admin" ? "This console is for administrators." : "Use the admin console for this account.");
      }
      client.setToken(result.access_token);
      setUser(result.user);
    },
    async logout() {
      await client.logout().catch(() => undefined);
      client.setToken(null);
      setUser(null);
    },
  };
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useSession() {
  const value = useContext(Ctx);
  if (!value) throw new Error("session missing");
  return value;
}
