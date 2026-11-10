import { test, expect } from "@playwright/test";
import { typeInto } from "./helpers/forms";

const adminBase = process.env.E2E_ADMIN_BASE_URL ?? "http://127.0.0.1:4322";

async function adminLogin(page: import("@playwright/test").Page) {
  const adminEmail = process.env.E2E_ADMIN_EMAIL;
  const adminPassword = process.env.E2E_ADMIN_PASSWORD;
  test.skip(!adminEmail || !adminPassword, "Set E2E_ADMIN_* in e2e/.env.local");
  await page.goto(`${adminBase}/login`);
  await typeInto(page.getByPlaceholder("Admin email"), adminEmail!);
  await typeInto(page.getByPlaceholder("Password"), adminPassword!);
  const loginResp = page.waitForResponse(
    (r) => r.url().includes("/api/v1/auth/login") && r.request().method() === "POST",
  );
  await page.getByRole("button", { name: "Sign in" }).click();
  expect((await loginResp).ok()).toBeTruthy();
}

test("overview shows Kill Switch without raw JSON", async ({ page }) => {
  await adminLogin(page);
  await page.goto(`${adminBase}/`);
  await expect(page.getByText("Kill Switch", { exact: true })).toBeVisible();
  await expect(page.getByText('{"enabled"')).not.toBeVisible();
});

test("kill switch toggle requires confirmation and single POST", async ({ page }) => {
  await adminLogin(page);
  await page.goto(`${adminBase}/settings`);

  const killPosts: string[] = [];
  page.on("request", (req) => {
    if (req.url().includes("/api/v1/admin/kill-switch") && req.method() === "POST") {
      killPosts.push(req.postData() ?? "");
    }
  });

  const toggle = page.getByRole("button", { name: /Turn kill switch/i });
  await toggle.click();
  await expect(page.getByRole("dialog")).toBeVisible();
  expect(killPosts.length).toBe(0);

  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(killPosts.length).toBe(0);

  await toggle.click();
  await page.getByRole("button", { name: /Turn Kill Switch (ON|OFF)/ }).click();
  await expect.poll(() => killPosts.length, { timeout: 10_000 }).toBe(1);
});
