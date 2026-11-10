import { test, expect } from "@playwright/test";
import { execSync } from "node:child_process";
import * as path from "node:path";
import { loginUser, typeInto } from "./helpers/forms";
import { apiLogin, apiPostJson } from "../helpers/api";

const adminEmail = process.env.E2E_ADMIN_EMAIL;
const adminPassword = process.env.E2E_ADMIN_PASSWORD;
const invitePassword = process.env.E2E_INVITED_USER_PASSWORD;
const adminBase = process.env.E2E_ADMIN_BASE_URL ?? "http://127.0.0.1:4322";
const apiBase = process.env.E2E_API_BASE_URL ?? "http://127.0.0.1:4321";
const repoRoot = path.resolve(__dirname, "../..");

/** API logins use a dedicated synthetic IP — not the shared browser "local" bucket. */
const PAPER_API_XFF = "203.0.113.30";

async function userToken(email: string, password: string) {
  return apiLogin(email, password, PAPER_API_XFF);
}

const stamp = Date.now();
const userEmail = `pw-paper-${stamp}@example.com`;
const wallet = "0x5555555555555555555555555555555555555555";
const signer = "0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc";

test.describe.configure({ mode: "serial", timeout: 300_000 });

test.beforeAll(() => {
  test.skip(
    !adminEmail || !adminPassword || !invitePassword,
    "Set E2E_ADMIN_* and E2E_INVITED_USER_PASSWORD in e2e/.env.local",
  );
});

function inject(side: string, suffix: string) {
  execSync(
    `INJECT_WALLET=${wallet} INJECT_SIDE=${side} INJECT_TX_SUFFIX=${suffix} bash scripts/inject-paper-event.sh`,
    {
      cwd: repoRoot,
      stdio: "pipe",
      env: { ...process.env, E2E_MODE: process.env.E2E_MODE ?? "1" },
    },
  );
}

test("full paper path: admin invite → user setup → strategy → mock trades → UI", async ({
  browser,
}) => {
  const admin = await browser.newPage();
  await admin.goto(`${adminBase}/login`);
  await typeInto(admin.getByPlaceholder("Admin email"), adminEmail!);
  const adminPwd = admin.getByPlaceholder("Password");
  await typeInto(adminPwd, adminPassword!);
  const adminLoginResp = admin.waitForResponse(
    (r) => r.url().includes("/api/v1/auth/login") && r.request().method() === "POST",
  );
  await admin.getByRole("button", { name: "Sign in" }).click();
  expect((await adminLoginResp).ok()).toBeTruthy();
  await admin.goto(`${adminBase}/users`);
  await typeInto(admin.getByPlaceholder("Email"), userEmail);
  await typeInto(admin.getByPlaceholder("Display name"), "Playwright Paper");
  await admin.getByRole("button", { name: "Create invitation" }).click();
  await expect(admin.getByText("Invitation link")).toBeVisible({ timeout: 30_000 });
  const setupText = await admin.locator("code").textContent();
  expect(setupText).toContain("/setup?token=");
  await admin.close();

  const user = await browser.newPage();
  await user.goto(setupText!.trim());
  await typeInto(user.locator('input[name="password"]'), invitePassword!);
  await user.getByRole("button", { name: "Save password" }).click();
  await expect(user).toHaveURL(/\/login/, { timeout: 15_000 });
  await loginUser(user, userEmail, invitePassword!);

  // Account form is uncontrolled; automation can miss fields — create via API, continue in UI.
  const token = await userToken(userEmail, invitePassword!);
  const acctRes = await fetch(`${apiBase}/api/v1/accounts`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      name: "Paper UI Account",
      signature_type: "eoa",
      signer_address: signer,
    }),
  });
  expect(acctRes.ok).toBeTruthy();
  const acct = (await acctRes.json()) as { id: string };
  const accountUrl = `/accounts/${acct.id}`;
  await user.goto(accountUrl);
  await expect(user.locator("h1")).toContainText("Paper UI Account");
  const modeRes = await fetch(`${apiBase}/api/v1/accounts/${acct.id}/mode`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ mode: "paper_trading" }),
  });
  expect(modeRes.ok).toBeTruthy();
  const riskRes = await fetch(`${apiBase}/api/v1/accounts/${acct.id}/risk`, {
    method: "PATCH",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      paper_cash_usd: "1000",
      max_order_usd: "100",
      max_exposure_usd: "1000",
    }),
  });
  expect(riskRes.ok).toBeTruthy();

  await user.goto("/traders/new");
  await typeInto(user.getByPlaceholder("Alias"), "PW Leader");
  await typeInto(user.getByPlaceholder("Public 0x address"), wallet);
  await user.getByRole("button", { name: "Check and add" }).click();
  await expect(user).toHaveURL(/\/traders\/[0-9a-f-]+/, { timeout: 30_000 });
  await expect(user.locator('select[name="trading_account_id"] option')).toHaveCount(1, {
    timeout: 30_000,
  });
  await user.locator('select[name="trading_account_id"]').selectOption({ label: "Paper UI Account" });
  await user.getByRole("button", { name: "Create disabled strategy" }).click();
  await expect(user).toHaveURL(/\/strategies\/[0-9a-f-]+/, { timeout: 15_000 });
  const strategyId = user.url().split("/").pop()!;

  await user.goto("/strategies");
  const row = user.locator("table tbody tr").first();
  await expect(row).toContainText("no");

  const enableRes = await fetch(`${apiBase}/api/v1/strategies/${strategyId}/enable`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ enabled: true }),
  });
  expect(enableRes.ok).toBeTruthy();

  let ordersRequest = "";
  user.on("request", (req) => {
    if (req.url().includes("/api/v1/orders") && req.method() === "GET") {
      ordersRequest = req.url();
    }
  });

  inject("buy", `${stamp}b1`);
  await user.goto("/orders");
  await expect(user.locator("table tbody tr")).toHaveCount(1, { timeout: 30_000 });
  const ordersUrl = new URL(ordersRequest || `${user.url()}`);
  expect(ordersUrl.searchParams.has("side")).toBe(false);
  expect(ordersUrl.searchParams.has("status")).toBe(false);

  inject("sell", `${stamp}s1`);
  await user.getByRole("button", { name: "Refresh" }).click();
  await expect(user.locator("table tbody tr")).toHaveCount(2, { timeout: 30_000 });

  const before = await user.locator("table tbody tr").count();
  inject("buy", `${stamp}b1`);
  await user.getByRole("button", { name: "Refresh" }).click();
  await expect(user.locator("table tbody tr")).toHaveCount(before, { timeout: 15_000 });

  await user.close();
});
