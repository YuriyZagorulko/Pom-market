import type {
  AdminOverview,
  DashboardSummary,
  OperatingMode,
  SourceTraderPageDetail,
  TradingAccountDetail,
} from "../types/platform";

export const API_BASE_URL =
  process.env.NEXT_PUBLIC_API_BASE_URL || "http://localhost:4320";

export const WS_BASE_URL =
  process.env.NEXT_PUBLIC_WS_BASE_URL || "ws://localhost:4320";

async function apiRequest<T>(
  path: string,
  token: string,
  options: RequestInit = {}
): Promise<T> {
  const res = await fetch(`${API_BASE_URL}${path}`, {
    ...options,
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${token}`,
      ...(options.headers || {}),
    },
  });
  if (!res.ok) {
    const body = await res.json().catch(() => ({ detail: res.statusText }));
    throw new Error(body.detail || `HTTP ${res.status}`);
  }
  return res.json() as Promise<T>;
}

export const PlatformApiClient = {
  getDashboardSummary(token: string): Promise<DashboardSummary> {
    return apiRequest<DashboardSummary>("/api/v1/dashboard/summary", token);
  },

  getAccountDetail(token: string, accountId: number): Promise<TradingAccountDetail> {
    return apiRequest<TradingAccountDetail>(
      `/api/v1/accounts/${accountId}/detail`,
      token
    );
  },

  setOperatingMode(
    token: string,
    accountId: number,
    operatingMode: OperatingMode,
    explicitConfirmation?: string
  ) {
    return apiRequest(`/api/v1/accounts/${accountId}/mode`, token, {
      method: "POST",
      body: JSON.stringify({
        operating_mode: operatingMode,
        explicit_confirmation: explicitConfirmation,
      }),
    });
  },

  setAccountEmergencyStop(
    token: string,
    accountId: number,
    emergencyStop: boolean,
    reason?: string
  ) {
    return apiRequest(`/api/v1/accounts/${accountId}/emergency-stop`, token, {
      method: "POST",
      body: JSON.stringify({
        emergency_stop: emergencyStop,
        reason,
      }),
    });
  },

  getSourceTraderPage(
    token: string,
    walletAddress: string
  ): Promise<SourceTraderPageDetail> {
    return apiRequest<SourceTraderPageDetail>(
      `/api/v1/source-traders/${walletAddress}`,
      token
    );
  },

  getAdminOverview(token: string): Promise<AdminOverview> {
    return apiRequest<AdminOverview>("/api/v1/admin/overview", token);
  },

  suspendUser(
    token: string,
    userId: number,
    isSuspended: boolean,
    reason?: string
  ) {
    return apiRequest(`/api/v1/admin/users/${userId}/suspend`, token, {
      method: "POST",
      body: JSON.stringify({ is_suspended: isSuspended, reason }),
    });
  },

  setGlobalEmergencyStop(token: string, enabled: boolean, reason: string) {
    return apiRequest("/api/v1/admin/emergency-stop", token, {
      method: "POST",
      body: JSON.stringify({ enabled, reason }),
    });
  },

  connectActivityWebSocket(onMessage: (data: unknown) => void): WebSocket {
    const ws = new WebSocket(`${WS_BASE_URL}/ws/activity`);
    ws.onmessage = (event) => {
      try {
        onMessage(JSON.parse(event.data));
      } catch {
        onMessage(event.data);
      }
    };
    return ws;
  },
};
