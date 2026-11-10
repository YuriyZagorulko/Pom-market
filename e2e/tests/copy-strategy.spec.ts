import { test, expect } from "@playwright/test";
import { injectPaperTrade } from "../helpers/inject";
import { loginUser, typeInto } from "./helpers/forms";
import { apiLogin } from "../helpers/api";

const adminEmail = process.env.E2E_ADMIN_EMAIL;
const adminPassword = process.env.E2E_ADMIN_PASSWORD;
const invitePassword = process.env.E2E_INVITED_USER_PASSWORD;
const adminBase = process.env.E2E_ADMIN_BASE_URL ?? "http://127.0.0.1:4322";
const apiBase = process.env.E2E_API_BASE_URL ?? "http://127.0.0.1:4321";
const PAPER_API_XFF = "203.0.113.31";

async function userToken(email: string, password: string) {
  return apiLogin(email, password, PAPER_API_XFF);
}

const stamp = Date.now();
const userEmail = `pw-copy-${stamp}@example.com`;
const wallet = `0x${(stamp % 1_0000_0000).toString(16).padStart(40, "0").slice(-40)}`;

test.describe.configure({ mode: "serial", timeout: 300_000 });

test.beforeAll(() => {
  test.skip(
    !adminEmail || !adminPassword || !invitePassword,
    "Set E2E_ADMIN_* and E2E_INVITED_USER_PASSWORD in e2e/.env.local",
  );
});

test("copy strategy wizard, preview, paper copies, utilization, live rejected", async ({ browser }) => {
  const admin = await browser.newPage();
  await admin.goto(`${adminBase}/login`);
  await typeInto(admin.getByPlaceholder("Admin email"), adminEmail!);
  await typeInto(admin.getByPlaceholder("Password"), adminPassword!);
  const adminLoginResp = admin.waitForResponse(
    (r) => r.url().includes("/api/v1/auth/login") && r.request().method() === "POST",
  );
  await admin.getByRole("button", { name: "Sign in" }).click();
  expect((await adminLoginResp).ok()).toBeTruthy();
  await admin.goto(`${adminBase}/users`);
  await typeInto(admin.getByPlaceholder("Email"), userEmail);
  await typeInto(admin.getByPlaceholder("Display name"), "Copy E2E");
  await admin.getByRole("button", { name: "Create invitation" }).click();
  await expect(admin.getByText("Invitation link")).toBeVisible({ timeout: 30_000 });
  const setupText = await admin.locator("code").textContent();
  await admin.close();

  const user = await browser.newPage();
  await user.goto(setupText!.trim());
  await typeInto(user.locator('input[name="password"]'), invitePassword!);
  await user.getByRole("button", { name: "Save password" }).click();
  await expect(user).toHaveURL(/\/login/, { timeout: 15_000 });
  await loginUser(user, userEmail, invitePassword!);

  const token = await userToken(userEmail, invitePassword!);
  const acctRes = await fetch(`${apiBase}/api/v1/accounts`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({
      name: "Copy Paper",
      signature_type: "eoa",
      signer_address: "0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc",
    }),
  });
  expect(acctRes.ok).toBeTruthy();
  const acct = (await acctRes.json()) as { id: string };
  await fetch(`${apiBase}/api/v1/accounts/${acct.id}/mode`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({ mode: "paper_trading" }),
  });
  await fetch(`${apiBase}/api/v1/accounts/${acct.id}/risk`, {
    method: "PATCH",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({
      paper_cash_usd: "5000",
      max_order_usd: "500",
      max_exposure_usd: "5000",
    }),
  });

  await user.goto("/traders/new");
  await typeInto(user.getByPlaceholder("Alias"), "E2E Leader");
  await typeInto(user.getByPlaceholder("Public 0x address"), wallet);
  await user.getByRole("button", { name: "Check and add" }).click();
  await expect(user).toHaveURL(/\/traders\/[0-9a-f-]+/, { timeout: 30_000 });
  const traderId = user.url().split("/").pop()!;

  await user.goto(`/traders/${traderId}/strategy/new`);
  await user.locator("select").first().selectOption({ label: "Copy Paper" });
  await user.getByRole("button", { name: "$2,000" }).click();
  await user.getByRole("button", { name: "10%" }).click();
  await user.getByPlaceholder("Max per trade USD").fill("200");
  await user.getByRole("button", { name: "Preview recent trades" }).click();
  await expect(user.getByRole("heading", { name: "Trade preview" })).toBeVisible({ timeout: 15_000 });

  await user.getByRole("button", { name: "Create disabled strategy" }).click();
  await user.waitForURL(/\/strategies\/[a-f0-9-]+/, { timeout: 15_000 });
  const strategyId = user.url().split("/").pop()!;
  await expect(user.getByRole("heading", { name: "Budget overview" })).toBeVisible();

  user.once("dialog", (d) => d.accept());
  await user.getByRole("button", { name: /Enable strategy/ }).click();
  await expect(user.getByText("Strategy enabled")).toBeVisible({ timeout: 10_000 });

  injectPaperTrade(wallet, "buy", `${stamp}-buy1k`);
  await user.waitForTimeout(3000);
  const utilRes = await fetch(`${apiBase}/api/v1/strategies/${strategyId}/utilization`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  expect(utilRes.ok).toBeTruthy();
  const util = (await utilRes.json()) as { deployed_capital_usd: string };
  expect(Number(util.deployed_capital_usd)).toBeGreaterThan(0);

  injectPaperTrade(wallet, "sell", `${stamp}-sell1`);
  await user.waitForTimeout(2000);

  const liveRes = await fetch(`${apiBase}/api/v1/strategies/${strategyId}/enable`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({ enabled: true }),
  });
  await fetch(`${apiBase}/api/v1/accounts/${acct.id}/mode`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({
      mode: "live_trading",
      confirm_live: true,
      confirmation: "ENABLE LIVE TRADING",
    }),
  });
  const liveEnable = await fetch(`${apiBase}/api/v1/strategies/${strategyId}/enable`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({ enabled: true }),
  });
  expect(liveEnable.status).toBeGreaterThanOrEqual(400);

  await user.close();
});
