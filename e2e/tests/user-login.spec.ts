import { test, expect } from "@playwright/test";
import { loginUser } from "./helpers/forms";
import { loadE2eUserCreds } from "../helpers/env";

test("user login via real input and protected page", async ({ page }) => {
  const creds = loadE2eUserCreds();
  test.skip(!creds, "global-setup did not provision E2E user (run with e2e/.env.local)");

  await loginUser(page, creds!.email, creds!.password);
  await page.goto("/accounts");
  await expect(page.getByRole("heading", { name: "Polymarket accounts" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Add account" })).toBeVisible();
});
