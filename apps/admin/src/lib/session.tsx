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

const SESSION_REFRESH_MS = 12 * 60 * 1000;

function applySession(
  client: ApiClient,
  role: "user" | "admin",
  result: { access_token: string; user: ApiUser },
  setUser: (u: ApiUser | null) => void,
) {
  if (result.user.role === role) {
    client.setToken(result.access_token);
    setUser(result.user);
  }
}

export function SessionProvider({ children, role }: { children: React.ReactNode; role: "user" | "admin" }) {
  const client = useMemo(() => new ApiClient(), []);
  const [user, setUser] = useState<ApiUser | null>(null);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    client.setOnRefreshed((accessToken, refreshedUser) => {
      applySession(client, role, { access_token: accessToken, user: refreshedUser }, setUser);
    });
    return () => client.setOnRefreshed(null);
  }, [client, role]);

  useEffect(() => {
    client.refresh().then((result) => {
      applySession(client, role, result, setUser);
    }).catch(() => undefined).finally(() => setReady(true));
  }, [client, role]);

  useEffect(() => {
    if (!user) return;
    const timer = setInterval(() => {
      client.refresh().then((result) => {
        applySession(client, role, result, setUser);
      }).catch(() => {
        client.setToken(null);
        setUser(null);
      });
    }, SESSION_REFRESH_MS);
    return () => clearInterval(timer);
  }, [client, role, user]);

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
