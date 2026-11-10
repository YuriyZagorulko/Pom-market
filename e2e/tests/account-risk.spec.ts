import { test, expect } from "@playwright/test";
import { apiLogin, apiPostJson } from "../helpers/api";

const apiBase = process.env.E2E_API_BASE_URL ?? "http://127.0.0.1:4324";
const adminEmail = process.env.E2E_ADMIN_EMAIL;
const adminPassword = process.env.E2E_ADMIN_PASSWORD;
const invitePassword = process.env.E2E_INVITED_USER_PASSWORD;
const XFF = "203.0.113.88";

test.beforeEach(() => {
  test.skip(
    !adminEmail || !adminPassword || !invitePassword,
    "Set E2E_ADMIN_*, E2E_INVITED_USER_PASSWORD",
  );
});

test("account risk PATCH accepts numeric slippage and persists", async () => {
  const adminToken = await apiLogin(adminEmail!, adminPassword!, XFF);
  const stamp = Date.now();
  const email = `risk-${stamp}@example.com`;
  const createUser = await fetch(`${apiBase}/api/v1/admin/users`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${adminToken}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ email, display_name: "RiskTest" }),
  });
  expect(createUser.ok).toBeTruthy();
  const setup = (await createUser.json()) as { setup_url: string };
  const token = setup.setup_url.split("token=")[1]!;
  await apiPostJson(
    "/api/v1/auth/password/setup",
    { token, password: invitePassword! },
    { forwardedFor: XFF },
  );
  const userToken = await apiLogin(email, invitePassword!, XFF);

  const acctRes = await fetch(`${apiBase}/api/v1/accounts`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${userToken}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      name: "Risk acct",
      signature_type: "eoa",
      signer_address: "0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc",
    }),
  });
  expect(acctRes.ok).toBeTruthy();
  const { id: accountId } = (await acctRes.json()) as { id: string };

  const patchBody = {
    max_order_usd: "50.00000000",
    max_daily_loss_usd: "100.00000000",
    max_exposure_usd: "500.00000000",
    max_slippage_bps: 150,
  };
  const patch = await fetch(`${apiBase}/api/v1/accounts/${accountId}/risk`, {
    method: "PATCH",
    headers: {
      Authorization: `Bearer ${userToken}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(patchBody),
  });
  expect(patch.ok).toBeTruthy();

  const fund = await fetch(`${apiBase}/api/v1/accounts/${accountId}/capital/paper-funding`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${userToken}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      delta_usd: "5000",
      reason: "e2e_risk_test",
      idempotency_key: `e2e-fund-${stamp}`,
    }),
  });
  expect(fund.ok).toBeTruthy();

  const get = await fetch(`${apiBase}/api/v1/accounts/${accountId}`, {
    headers: { Authorization: `Bearer ${userToken}` },
  });
  expect(get.ok).toBeTruthy();
  const detail = (await get.json()) as { risk: { max_slippage_bps: number; paper_cash_usd: string } };
  expect(Number(detail.risk.max_slippage_bps)).toBe(150);
  expect(String(detail.risk.paper_cash_usd)).toMatch(/5000/);
});

test("account risk PATCH still accepts string slippage for backward compatibility", async () => {
  const adminToken = await apiLogin(adminEmail!, adminPassword!, XFF);
  const stamp = Date.now();
  const email = `risk-str-${stamp}@example.com`;
  const createUser = await fetch(`${apiBase}/api/v1/admin/users`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${adminToken}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ email, display_name: "RiskStr" }),
  });
  const setup = (await createUser.json()) as { setup_url: string };
  const token = setup.setup_url.split("token=")[1]!;
  await apiPostJson(
    "/api/v1/auth/password/setup",
    { token, password: invitePassword! },
    { forwardedFor: XFF },
  );
  const userToken = await apiLogin(email, invitePassword!, XFF);
  const acctRes = await fetch(`${apiBase}/api/v1/accounts`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${userToken}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      name: "Risk2",
      signature_type: "eoa",
      signer_address: "0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65",
    }),
  });
  const { id: accountId } = (await acctRes.json()) as { id: string };
  const patch = await fetch(`${apiBase}/api/v1/accounts/${accountId}/risk`, {
    method: "PATCH",
    headers: {
      Authorization: `Bearer ${userToken}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      max_slippage_bps: "200",
    }),
  });
  expect(patch.ok).toBeTruthy();

  const badCash = await fetch(`${apiBase}/api/v1/accounts/${accountId}/risk`, {
    method: "PATCH",
    headers: {
      Authorization: `Bearer ${userToken}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ paper_cash_usd: "100" }),
  });
  expect(badCash.status).toBe(400);
});
