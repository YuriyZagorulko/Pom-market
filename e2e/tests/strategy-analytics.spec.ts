import { test, expect } from "@playwright/test";
import { loginUser } from "./helpers/forms";
import { loadE2eUserCreds } from "../helpers/env";

test("strategy analytics tabs and overview", async ({ page }) => {
  const creds = loadE2eUserCreds();
  if (!creds) {
    test.skip();
    return;
  }
  await loginUser(page, creds.email, creds.password);
  await page.goto("/strategies");
  await expect(page.getByRole("heading", { name: "Copy strategies" })).toBeVisible({ timeout: 15_000 });
  const link = page.locator("table tbody a[href^='/strategies/']").first();
  if (!(await link.isVisible())) {
    test.skip(true, "no strategies in e2e user");
    return;
  }
  await link.click();
  await expect(page.getByRole("button", { name: "Overview" })).toBeVisible({ timeout: 15_000 });
  await expect(page.getByText(/Partial equity/i).first()).toBeVisible({ timeout: 15_000 });
  for (const p of ["1D", "1W", "1M", "ALL"]) {
    await page.getByRole("button", { name: p, exact: true }).click();
  }
  await page.getByRole("button", { name: "Positions" }).click();
  await page.getByRole("button", { name: "Closed" }).click();
  await page.getByRole("button", { name: "Open" }).click();
  await page.getByRole("button", { name: "Activity" }).click();
  await page.getByRole("button", { name: "Performance" }).click();
  await page.getByRole("button", { name: "Settings" }).click();
  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
});
