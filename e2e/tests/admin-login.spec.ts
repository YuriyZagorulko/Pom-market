import { test, expect } from "@playwright/test";
import { loginAdmin } from "./helpers/forms";

const email = process.env.E2E_ADMIN_EMAIL;
const password = process.env.E2E_ADMIN_PASSWORD;

test.beforeEach(() => {
  test.skip(!email || !password, "Set E2E_ADMIN_EMAIL and E2E_ADMIN_PASSWORD in e2e/.env.local");
});

test("admin login via real input and protected page", async ({ page }) => {
  await loginAdmin(page, email!, password!);
  await page.goto("/users");
  await expect(page.getByRole("heading", { name: "Users" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Create invitation" })).toBeVisible();
});
