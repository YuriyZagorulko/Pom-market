const apiBase = process.env.E2E_API_BASE_URL ?? "http://127.0.0.1:4321";

export async function apiPostJson(
  path: string,
  body: unknown,
  opts?: { token?: string; forwardedFor?: string },
) {
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (opts?.token) headers.Authorization = `Bearer ${opts.token}`;
  // Direct API calls only: affects buckets when TRUSTED_PROXIES includes the test runner (e.g. 127.0.0.0/8).
  if (opts?.forwardedFor) headers["X-Forwarded-For"] = opts.forwardedFor;
  return fetch(`${apiBase}${path}`, { method: "POST", headers, body: JSON.stringify(body) });
}

export async function apiLogin(email: string, password: string, forwardedFor: string) {
  const res = await apiPostJson("/api/v1/auth/login", { email, password }, { forwardedFor });
  if (!res.ok) {
    throw new Error(`login failed: HTTP ${res.status}`);
  }
  const data = (await res.json()) as { access_token: string };
  return data.access_token;
}
