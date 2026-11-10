import { test, expect } from "@playwright/test";
import { typeInto, loginUser } from "./helpers/forms";
import { apiLogin, apiPostJson } from "../helpers/api";
import { injectPaperTrade } from "../helpers/inject";

const adminEmail = process.env.E2E_ADMIN_EMAIL;
const adminPassword = process.env.E2E_ADMIN_PASSWORD;
const invitePassword = process.env.E2E_INVITED_USER_PASSWORD;
const adminBase = process.env.E2E_ADMIN_BASE_URL ?? "http://127.0.0.1:4325";
const apiBase = process.env.E2E_API_BASE_URL ?? "http://127.0.0.1:4324";
const API_XFF = "203.0.113.55";
const stamp = Date.now();
const userEmail = `pw-danger-${stamp}@example.com`;
const wallet = "0x7777777777777777777777777777777777777777";
const signer = "0x9965507D1a55bcC2695C58ba16Fa3C528af0BbFc";

test.describe.configure({ mode: "serial", timeout: 300_000 });

test.beforeAll(() => {
  test.skip(
    !adminEmail || !adminPassword || !invitePassword,
    "Set E2E_ADMIN_* and E2E_INVITED_USER_PASSWORD",
  );
});

async function bearer(email: string, password: string) {
  return apiLogin(email, password, API_XFF);
}

test("danger zone: controls, hard close, resume", async ({ browser }) => {
  const admin = await browser.newPage();
  await admin.goto(`${adminBase}/login`);
  await typeInto(admin.getByPlaceholder("Admin email"), adminEmail!);
  await typeInto(admin.getByPlaceholder("Password"), adminPassword!);
  const adminLogin = admin.waitForResponse(
    (r) => r.url().includes("/api/v1/auth/login") && r.request().method() === "POST",
  );
  await admin.getByRole("button", { name: "Sign in" }).click();
  expect((await adminLogin).ok()).toBeTruthy();
  await admin.goto(`${adminBase}/users`);
  await typeInto(admin.getByPlaceholder("Email"), userEmail);
  await typeInto(admin.getByPlaceholder("Display name"), "Danger E2E");
  await admin.getByRole("button", { name: "Create invitation" }).click();
  await expect(admin.getByText("Invitation link")).toBeVisible({ timeout: 30_000 });
  const setupText = await admin.locator("code").textContent();
  await admin.close();

  const page = await browser.newPage();
  await page.goto(setupText!.trim());
  await typeInto(page.locator('input[name="password"]'), invitePassword!);
  await page.getByRole("button", { name: "Save password" }).click();
  await expect(page).toHaveURL(/\/login/, { timeout: 15_000 });
  await loginUser(page, userEmail, invitePassword!);
  const token = await bearer(userEmail, invitePassword!);

  const acctRes = await fetch(`${apiBase}/api/v1/accounts`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({
      name: "Danger Paper",
      signature_type: "eoa",
      signer_address: signer,
    }),
  });
  expect(acctRes.ok).toBeTruthy();
  const acct = (await acctRes.json()) as { id: string };
  const accountId = acct.id;

  await fetch(`${apiBase}/api/v1/accounts/${accountId}/mode`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({ mode: "paper_trading" }),
  });
  await fetch(`${apiBase}/api/v1/accounts/${accountId}/risk`, {
    method: "PATCH",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({ max_order_usd: "200", max_exposure_usd: "5000" }),
  });
  await fetch(`${apiBase}/api/v1/accounts/${accountId}/capital/paper-funding`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({
      delta_usd: "2000",
      reason: "e2e_danger_zone",
      idempotency_key: `e2e-danger-fund-${stamp}`,
    }),
  });

  await page.goto(`/accounts/${accountId}`);
  await expect(page.getByRole("heading", { name: "Danger Paper" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Danger zone" })).toBeVisible();
  await expect(page.getByText(/Current state:/i)).toContainText("normal");

  page.once("dialog", (d) => {
    expect(d.type()).toBe("confirm");
    d.accept();
  });
  await page.getByRole("button", { name: "Activate kill switch" }).click();
  await expect
    .poll(async () => {
      const res = await fetch(`${apiBase}/api/v1/accounts/${accountId}/trading-control`, {
        headers: { Authorization: `Bearer ${token}` },
      });
      const body = (await res.json()) as { trading_control_state: string };
      return body.trading_control_state;
    })
    .toBe("kill_switch");
  await expect(page.getByText(/Current state:/i)).toContainText("kill switch");

  page.once("dialog", (d) => d.accept());
  await page.getByRole("button", { name: "Deactivate kill switch" }).click();
  await expect
    .poll(async () => {
      const res = await fetch(`${apiBase}/api/v1/accounts/${accountId}/trading-control`, {
        headers: { Authorization: `Bearer ${token}` },
      });
      const body = (await res.json()) as { trading_control_state: string };
      return body.trading_control_state;
    })
    .toBe("normal");

  page.once("dialog", (d) => d.accept());
  await page.getByRole("button", { name: "Activate soft close" }).click();
  await expect
    .poll(async () => {
      const res = await fetch(`${apiBase}/api/v1/accounts/${accountId}/trading-control`, {
        headers: { Authorization: `Bearer ${token}` },
      });
      const body = (await res.json()) as { trading_control_state: string; blocks_buy: boolean };
      return body.trading_control_state === "soft_close" && body.blocks_buy;
    })
    .toBe(true);
  const softBody = (await (
    await fetch(`${apiBase}/api/v1/accounts/${accountId}/trading-control`, {
      headers: { Authorization: `Bearer ${token}` },
    })
  ).json()) as { allows_copy_sell: boolean };
  expect(softBody.allows_copy_sell).toBe(true);

  const traderRes = await apiPostJson(
    `/api/v1/source-traders`,
    { wallet_address: wallet, alias: "DZ Leader" },
    { token, forwardedFor: API_XFF },
  );
  expect(traderRes.ok).toBeTruthy();
  const traderBody = (await traderRes.json()) as { id: string };
  const traderId = traderBody.id;
  const stratRes = await apiPostJson(
    `/api/v1/strategies`,
    {
      trading_account_id: accountId,
      source_trader_id: traderId,
      sizing_mode: "proportional",
      copy_percent: "10",
      strategy_budget_usd: "500",
      max_position_usd: "200",
      max_exposure_usd: "2000",
      max_open_positions: 10,
      max_slippage_bps: 500,
      min_order_usd: "1",
    },
    { token, forwardedFor: API_XFF },
  );
  expect(stratRes.ok).toBeTruthy();
  const strategy = (await stratRes.json()) as { id: string };
  const enableRes = await apiPostJson(
    `/api/v1/strategies/${strategy.id}/enable`,
    { enabled: true },
    { token, forwardedFor: API_XFF },
  );
  expect(enableRes.ok).toBeTruthy();

  injectPaperTrade(wallet, "buy", `dz${stamp}`);

  await page.getByRole("button", { name: "Load close preview" }).click();
  await expect(page.locator("pre")).toBeVisible({ timeout: 15_000 });

  await typeInto(page.getByPlaceholder("Type CLOSE ALL"), "CLOSE");
  await page.getByRole("button", { name: "Hard close all positions" }).click();
  await expect(page.locator("p.error", { hasText: "Type CLOSE ALL to confirm" })).toBeVisible();

  await typeInto(page.getByPlaceholder("Type CLOSE ALL"), "CLOSE ALL");
  const hcPosts: string[] = [];
  page.on("request", (req) => {
    if (req.url().includes("/hard-close") && req.method() === "POST" && !req.url().includes("/retry")) {
      hcPosts.push(req.url());
    }
  });
  await page.getByRole("button", { name: "Hard close all positions" }).click();
  await expect.poll(() => hcPosts.length, { timeout: 15_000 }).toBe(1);
  await expect(page.getByText(/Operation /)).toBeVisible({ timeout: 60_000 });

  await expect
    .poll(
      async () => {
        const res = await fetch(`${apiBase}/api/v1/accounts/${accountId}/trading-control`, {
          headers: { Authorization: `Bearer ${token}` },
        });
        const body = (await res.json()) as {
          trading_control_state: string;
          resume_requires_confirmation: boolean;
        };
        return `${body.trading_control_state}:${body.resume_requires_confirmation}`;
      },
      { timeout: 120_000 },
    )
    .toMatch(/hard_close_(completed|partial|in_progress):true/);

  page.once("dialog", (d) => d.accept());
  const resumeVisible = page.getByRole("button", { name: "Resume normal trading" });
  if (await resumeVisible.isVisible()) {
    await resumeVisible.click();
    await expect
      .poll(async () => {
        const res = await fetch(`${apiBase}/api/v1/accounts/${accountId}/trading-control`, {
          headers: { Authorization: `Bearer ${token}` },
        });
        const body = (await res.json()) as { trading_control_state: string };
        return body.trading_control_state;
      })
      .toBe("normal");
  }

  await page.close();
});
