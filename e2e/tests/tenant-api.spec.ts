import { test, expect } from "@playwright/test";
import { apiLogin, apiPostJson } from "../helpers/api";

const apiBase = process.env.E2E_API_BASE_URL ?? "http://127.0.0.1:4321";
const adminEmail = process.env.E2E_ADMIN_EMAIL;
const adminPassword = process.env.E2E_ADMIN_PASSWORD;
const invitePassword = process.env.E2E_INVITED_USER_PASSWORD;

/** Synthetic IPs per E2E suite — avoids sharing the browser bucket key "local". */
const TENANT_XFF = "203.0.113.20";

test.beforeEach(() => {
  test.skip(
    !adminEmail || !adminPassword || !invitePassword,
    "Set E2E_ADMIN_*, E2E_INVITED_USER_PASSWORD",
  );
});

test("tenant isolation on live API", async () => {
  const adminToken = await apiLogin(adminEmail!, adminPassword!, TENANT_XFF);
  const stamp = Date.now();
  const emailA = `iso-a-${stamp}@example.com`;
  const emailB = `iso-b-${stamp}@example.com`;

  const create = async (email: string, name: string) => {
    const res = await fetch(`${apiBase}/api/v1/admin/users`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${adminToken}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({ email, display_name: name }),
    });
    expect(res.ok).toBeTruthy();
    const data = (await res.json()) as { setup_url: string };
    const token = data.setup_url.split("token=")[1]!;
    await apiPostJson(
      "/api/v1/auth/password/setup",
      { token, password: invitePassword! },
      { forwardedFor: TENANT_XFF },
    );
    return apiLogin(email, invitePassword!, TENANT_XFF);
  };

  const tokenA = await create(emailA, "IsoA");
  const tokenB = await create(emailB, "IsoB");

  const acctRes = await fetch(`${apiBase}/api/v1/accounts`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${tokenB}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      name: "B only",
      signature_type: "eoa",
      signer_address: "0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65",
    }),
  });
  expect(acctRes.ok).toBeTruthy();
  const acct = (await acctRes.json()) as { id: string };
  const acctId = acct.id;

  const listA = await fetch(`${apiBase}/api/v1/accounts`, {
    headers: { Authorization: `Bearer ${tokenA}` },
  });
  expect(listA.ok).toBeTruthy();
  const rows = (await listA.json()) as unknown[];
  expect(rows).toHaveLength(0);

  const peek = await fetch(`${apiBase}/api/v1/accounts/${acctId}`, {
    headers: { Authorization: `Bearer ${tokenA}` },
  });
  expect(peek.status).toBe(404);
});
