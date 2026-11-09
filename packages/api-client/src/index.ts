export type ApiUser = {
  id: string;
  email: string;
  display_name: string;
  role: string;
  status: string;
};

export class ApiError extends Error {
  constructor(public status: number, message: string) {
    super(message);
  }
}

export class ApiClient {
  constructor(private token: string | null = null) {}

  setToken(token: string | null) {
    this.token = token;
  }

  async request<T>(path: string, init: RequestInit = {}): Promise<T> {
    const headers = new Headers(init.headers);
    if (init.body && !headers.has("Content-Type")) headers.set("Content-Type", "application/json");
    if (this.token) headers.set("Authorization", `Bearer ${this.token}`);
    const response = await fetch(path, { ...init, headers, credentials: "include" });
    const text = await response.text();
    const body = text ? JSON.parse(text) : null;
    if (!response.ok) throw new ApiError(response.status, body?.error || response.statusText);
    return body as T;
  }

  login(email: string, password: string) {
    return this.request<{ access_token: string; user: ApiUser }>("/api/v1/auth/login", {
      method: "POST",
      body: JSON.stringify({ email, password }),
    });
  }

  refresh() {
    return this.request<{ access_token: string; user: ApiUser }>("/api/v1/auth/refresh", { method: "POST" });
  }

  logout() {
    return this.request("/api/v1/auth/logout", { method: "POST" });
  }
}
