import { test, expect } from "@playwright/test";
import { loginUser, typeInto } from "./helpers/forms";
import { loadE2eUserCreds } from "../helpers/env";
import { apiLogin, apiPostJson } from "../helpers/api";
import { injectPaperTrade } from "../helpers/inject";
const PAPER_API_XFF = "203.0.113.40";
const apiBase = () => process.env.E2E_API_BASE_URL ?? "http://127.0.0.1:4324";

test.describe.configure({ mode: "serial", timeout: 180_000 });

function creds() {
  const c = loadE2eUserCreds();
  if (!c) throw new Error("missing E2E user — global-setup did not write e2e/.env.user.local");
  return c;
}

async function login(page: import("@playwright/test").Page) {
  const { email, password } = creds();
  await loginUser(page, email, password);
}

async function apiToken() {
  const { email, password } = creds();
  return apiLogin(email, password, PAPER_API_XFF);
}

async function ensureWalletTrade(): Promise<string | null> {
  const workerUrl = process.env.WORKER_DATABASE_URL;
  if (!workerUrl?.includes("pommarket_e2e")) {
    return null;
  }
  const token = await apiToken();
  const wallet = "0x6666666666666666666666666666666666666666";
  const traderRes = await apiPostJson(
    "/api/v1/source-traders",
    { wallet_address: wallet, alias: "E2E Trading" },
    { token, forwardedFor: PAPER_API_XFF },
  );
  if (!traderRes.ok) return null;
  const trader = (await traderRes.json()) as { id: string };
  try {
    injectPaperTrade(wallet, "buy", `e2etd${Date.now()}`);
  } catch {
    return trader.id;
  }
  return trader.id;
}

test("wallet activity: filters, presets, URL reload", async ({ page }) => {
  const traderId = await ensureWalletTrade();

  await login(page);
  await page.goto("/trades");
  await expect(page.getByRole("heading", { name: "Wallet activity" })).toBeVisible();
  await expect(page.locator(".wallet-activity")).toBeVisible();
  if (traderId) {
    await expect
      .poll(async () => (await page.locator(".activity-card").count()) > 0, { timeout: 60_000 })
      .toBe(true);
  }

  await page
    .locator(".wallet-activity select")
    .filter({ has: page.locator('option[value="buy"]') })
    .first()
    .selectOption("buy");
  await page.waitForURL(/side=buy/);

  const search = page.getByPlaceholder(/search market/i);
  await typeInto(search, "poly");
  await search.press("Enter");
  await expect(page).toHaveURL(/q=poly/);

  await page.getByRole("button", { name: "7 days" }).click();
  await expect(page).toHaveURL(/from=/);
  await expect(page).toHaveURL(/to=/);

  const activityType = page.locator('select').filter({ has: page.locator('option[value="trade"]') });
  if (await activityType.count()) {
    await activityType.first().selectOption("trade");
    await expect(page).toHaveURL(/activity_type=trade/);
  }

  if (await page.getByRole("button", { name: "Clear all" }).isVisible()) {
    await page.getByRole("button", { name: "Clear all" }).click();
    await expect(page).not.toHaveURL(/side=buy/);
  }

  await page.getByRole("button", { name: /wallet/i }).first().click();
  const panel = page.locator(".wallet-multiselect-panel");
  if (await panel.isVisible()) {
    await panel.getByRole("button", { name: "Select all" }).click();
    await expect(panel.locator('input[type="checkbox"]').first()).toBeChecked();
  }

  const detail = page.locator("a[href^='/trades/']").first();
  if (await detail.isVisible()) {
    await detail.click();
    await expect(page).toHaveURL(/\/trades\/[0-9a-f-]+/);
    await expect(page.locator(".trade-detail-hero").first()).toBeVisible();
  } else {
    await page.goto("/trades");
  }

  await page.goto("/trades?side=buy&limit=25");
  await page.reload();
  await expect(page).toHaveURL(/side=buy/);
});

test("orders, executions, positions: account picker", async ({ page }) => {
  await login(page);
  await page.goto("/orders");
  await expect(page.getByRole("heading", { name: "Orders" })).toBeVisible();
  const account = page.getByLabel("Trading account");
  if ((await account.locator("option").count()) > 1) {
    await account.selectOption({ index: 1 });
    await expect(page).toHaveURL(/account_id=/);
    await page.reload();
    await expect(page).toHaveURL(/account_id=/);
  }

  const side = page.locator("select").filter({ has: page.locator('option[value="filled"]') }).first();
  if (await side.count()) {
    await side.selectOption("filled");
    await expect(page).toHaveURL(/status=filled/);
  }

  await page.goto("/executions");
  await expect(page.getByRole("heading", { name: "Executions" })).toBeVisible();

  await page.goto("/positions");
  await expect(page.getByRole("heading", { name: "Positions" })).toBeVisible();
});

test("activity: kind and date presets", async ({ page }) => {
  await login(page);
  await page.goto("/activity");
  await expect(page.getByRole("heading", { name: "Activity" })).toBeVisible();
  await typeInto(page.getByPlaceholder("Event type"), "order");
  await page.getByPlaceholder("Event type").blur();
  await expect(page).toHaveURL(/kind=order/);
  await page.getByRole("button", { name: "All time" }).click();
  await expect(page).not.toHaveURL(/from=/);
});

test("empty results and loading settles", async ({ page }) => {
  await login(page);
  await page.goto("/trades?q=zzzz_no_match_zzzz");
  await expect(page.getByText("Loading…")).toBeHidden({ timeout: 30_000 });
  await expect(page.locator(".activity-empty, .muted").first()).toBeVisible({ timeout: 15_000 });
});
