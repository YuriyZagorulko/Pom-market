import { apiLogin } from "./api";

const SETUP_XFF = "203.0.113.11";

export async function setGlobalKillSwitch(enabled: boolean) {
  const apiBase = process.env.E2E_API_BASE_URL ?? "http://127.0.0.1:4324";
  const adminEmail = process.env.E2E_ADMIN_EMAIL;
  const adminPassword = process.env.E2E_ADMIN_PASSWORD;
  if (!adminEmail || !adminPassword) return;
  const token = await apiLogin(adminEmail, adminPassword, SETUP_XFF);
  const res = await fetch(`${apiBase}/api/v1/admin/kill-switch`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
      "X-Forwarded-For": SETUP_XFF,
    },
    body: JSON.stringify({ enabled }),
  });
  if (!res.ok) {
    throw new Error(`setGlobalKillSwitch(${enabled}) failed: HTTP ${res.status}`);
  }
}
