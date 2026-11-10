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

export class ApiClient {
  constructor(private token: string | null = null) {}

  setToken(token: string | null) {
    this.token = token;
  }

  async request<T>(path: string, init: RequestInit = {}): Promise<T> {
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
    return this.request<{ access_token: string; user: ApiUser }>("/api/v1/auth/refresh", { method: "POST" });
  }

  logout() {
    return this.request("/api/v1/auth/logout", { method: "POST" });
  }
}
