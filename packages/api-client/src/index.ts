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

function parseResponseJson(text: string): unknown | null {
  if (!text.trim()) return null;
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return null;
  }
}

function errorObject(json: unknown): Record<string, unknown> | null {
  if (json && typeof json === "object" && !Array.isArray(json)) {
    return json as Record<string, unknown>;
  }
  return null;
}

function errorMessageFromResponse(status: number, statusText: string, text: string, json: Record<string, unknown> | null): string {
  const fromJson = json?.error;
  if (typeof fromJson === "string" && fromJson.trim()) return fromJson;
  const trimmed = text.trim();
  if (trimmed && trimmed.length <= 500 && !trimmed.startsWith("<")) return trimmed;
  if (status === 0 || statusText === "Failed to fetch") {
    return "Could not reach the API. Is the backend running on port 4321?";
  }
  return statusText || `Request failed (${status})`;
}

const AUTH_NO_RETRY = ["/api/v1/auth/login", "/api/v1/auth/refresh", "/api/v1/auth/logout"];

export class ApiClient {
  private refreshInFlight: Promise<{ access_token: string; user: ApiUser }> | null = null;
  private onRefreshed: ((accessToken: string, user: ApiUser) => void) | null = null;

  constructor(private token: string | null = null) {}

  setToken(token: string | null) {
    this.token = token;
  }

  setOnRefreshed(handler: ((accessToken: string, user: ApiUser) => void) | null) {
    this.onRefreshed = handler;
  }

  private async refreshAccessToken(): Promise<{ access_token: string; user: ApiUser }> {
    if (!this.refreshInFlight) {
      this.refreshInFlight = this.request<{ access_token: string; user: ApiUser }>(
        "/api/v1/auth/refresh",
        { method: "POST" },
        { allowUnauthorizedRetry: false },
      ).finally(() => {
        this.refreshInFlight = null;
      });
    }
    return this.refreshInFlight;
  }

  async request<T>(
    path: string,
    init: RequestInit = {},
    options: { allowUnauthorizedRetry?: boolean } = {},
  ): Promise<T> {
    const allowUnauthorizedRetry = options.allowUnauthorizedRetry !== false;
    const headers = new Headers(init.headers);
    if (init.body && !headers.has("Content-Type")) headers.set("Content-Type", "application/json");
    if (this.token) headers.set("Authorization", `Bearer ${this.token}`);
    let response: Response;
    let text = "";
    try {
      response = await fetch(path, { ...init, headers, credentials: "include" });
      text = await response.text();
    } catch {
      throw new ApiError(0, "Could not reach the API. Is the backend running on port 4321?");
    }
    const parsed = parseResponseJson(text);
    if (
      response.status === 401 &&
      allowUnauthorizedRetry &&
      this.token &&
      !AUTH_NO_RETRY.some((p) => path === p || path.startsWith(`${p}?`))
    ) {
      try {
        const refreshed = await this.refreshAccessToken();
        this.token = refreshed.access_token;
        this.onRefreshed?.(refreshed.access_token, refreshed.user);
        return this.request<T>(path, init, { allowUnauthorizedRetry: false });
      } catch {
        // fall through: expired access and refresh failed
      }
    }
    if (!response.ok) {
      throw new ApiError(
        response.status,
        errorMessageFromResponse(response.status, response.statusText, text, errorObject(parsed)),
      );
    }
    if (parsed !== null) return parsed as T;
    if (!text.trim()) return null as T;
    throw new ApiError(response.status, "Server returned a non-JSON response.");
  }

  login(email: string, password: string) {
    return this.request<{ access_token: string; user: ApiUser }>("/api/v1/auth/login", {
      method: "POST",
      body: JSON.stringify({ email, password }),
    });
  }

  refresh() {
    return this.refreshAccessToken();
  }

  logout() {
    return this.request("/api/v1/auth/logout", { method: "POST" });
  }
}
