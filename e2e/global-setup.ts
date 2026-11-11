import * as fs from "node:fs";
import * as path from "node:path";
import { loadDotEnvFile } from "./helpers/env";
import { apiLogin, apiPostJson } from "./helpers/api";
import { assertE2eApiHealth } from "./helpers/db";

/** Isolated synthetic client IP — separate from browser logins (key "local"). */
const SETUP_XFF = "203.0.113.10";

export default async function globalSetup() {
  const e2eDir = path.join(__dirname);
  loadDotEnvFile(path.join(e2eDir, ".env.local"));

  const apiBase = process.env.E2E_API_BASE_URL ?? "http://127.0.0.1:4324";
  if (process.env.E2E_USE_ISOLATED_DB === "1") {
    await assertE2eApiHealth(apiBase);
  }

  const adminEmail = process.env.E2E_ADMIN_EMAIL;
  const adminPassword = process.env.E2E_ADMIN_PASSWORD;
  const invitePassword = process.env.E2E_INVITED_USER_PASSWORD ?? "playwright-invite-10";

  if (!adminEmail || !adminPassword) {
    console.warn("global-setup: skip user provision (missing E2E_ADMIN_* in e2e/.env.local)");
    return;
  }

  const userEmail = `e2e-user-login-${Date.now()}@example.com`;
  const userPassword = invitePassword;

  const adminToken = await apiLogin(adminEmail, adminPassword, SETUP_XFF);

  const killOff = await fetch(`${apiBase}/api/v1/admin/kill-switch`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${adminToken}`,
      "Content-Type": "application/json",
      "X-Forwarded-For": SETUP_XFF,
    },
    body: JSON.stringify({ enabled: false }),
  });
  if (!killOff.ok) {
    throw new Error(`failed to disable global kill switch for E2E: HTTP ${killOff.status}`);
  }

  const created = await fetch(
    `${apiBase}/api/v1/admin/users`,
    {
      method: "POST",
      headers: {
        Authorization: `Bearer ${adminToken}`,
        "Content-Type": "application/json",
        "X-Forwarded-For": SETUP_XFF,
      },
      body: JSON.stringify({
        email: userEmail,
        display_name: "E2E User Login",
      }),
    },
  );
  if (!created.ok) {
    throw new Error(`invite user failed: HTTP ${created.status}`);
  }
  const body = (await created.json()) as { setup_url: string };
  const setupToken = body.setup_url.split("token=")[1];
  if (!setupToken) throw new Error("invite missing setup token");

  const setup = await apiPostJson(
    "/api/v1/auth/password/setup",
    { token: setupToken, password: userPassword },
    { forwardedFor: SETUP_XFF },
  );
  if (!setup.ok) throw new Error(`password setup failed: HTTP ${setup.status}`);

  const userToken = await apiLogin(userEmail, userPassword, SETUP_XFF);
  const stamp = Date.now();
  const wallet = `0x${stamp.toString(16).padStart(40, "0").slice(-40)}`;
  const signer = "0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc";

  const acctRes = await fetch(`${apiBase}/api/v1/accounts`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${userToken}`,
      "Content-Type": "application/json",
      "X-Forwarded-For": SETUP_XFF,
    },
    body: JSON.stringify({
      name: "E2E Analytics Paper",
      signature_type: "eoa",
      signer_address: signer,
    }),
  });
  if (!acctRes.ok) throw new Error(`global-setup account create failed: HTTP ${acctRes.status}`);
  const acct = (await acctRes.json()) as { id: string };

  async function userApi(method: string, path: string, body?: unknown) {
    return fetch(`${apiBase}${path}`, {
      method,
      headers: {
        Authorization: `Bearer ${userToken}`,
        "Content-Type": "application/json",
        "X-Forwarded-For": SETUP_XFF,
      },
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
  }

  const modeRes = await userApi("POST", `/api/v1/accounts/${acct.id}/mode`, { mode: "paper_trading" });
  if (!modeRes.ok) throw new Error(`global-setup paper mode failed: HTTP ${modeRes.status}`);

  const fundRes = await userApi("POST", `/api/v1/accounts/${acct.id}/capital/paper-funding`, {
    delta_usd: "2000",
    reason: "e2e_global_setup",
    idempotency_key: `e2e-global-fund-${stamp}`,
  });
  if (!fundRes.ok) throw new Error(`global-setup paper funding failed: HTTP ${fundRes.status}`);

  const traderRes = await apiPostJson(
    "/api/v1/source-traders",
    { wallet_address: wallet, alias: "E2E Analytics Leader" },
    { token: userToken, forwardedFor: SETUP_XFF },
  );
  if (!traderRes.ok) throw new Error(`global-setup source trader failed: HTTP ${traderRes.status}`);
  const trader = (await traderRes.json()) as { id: string };

  const stratRes = await apiPostJson(
    "/api/v1/strategies",
    {
      trading_account_id: acct.id,
      source_trader_id: trader.id,
      sizing_mode: "proportional",
      copy_percent: "10",
      strategy_budget_usd: "500",
      max_position_usd: "100",
      max_exposure_usd: "2000",
      max_open_positions: 5,
      max_slippage_bps: 500,
      min_order_usd: "1",
    },
    { token: userToken, forwardedFor: SETUP_XFF },
  );
  if (!stratRes.ok) throw new Error(`global-setup strategy create failed: HTTP ${stratRes.status}`);

  const out = path.join(e2eDir, ".env.user.local");
  fs.writeFileSync(
    out,
    [
      "# Generated by e2e/global-setup.ts — synthetic user, do not commit.",
      `E2E_USER_EMAIL=${userEmail}`,
      `E2E_USER_PASSWORD=${userPassword}`,
      "",
    ].join("\n"),
    { mode: 0o600 },
  );
  process.env.E2E_USER_EMAIL = userEmail;
  process.env.E2E_USER_PASSWORD = userPassword;
}
