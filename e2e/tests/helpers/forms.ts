import type { Locator, Page } from "@playwright/test";
import { expect } from "@playwright/test";

/** Real keystrokes — required for uncontrolled React inputs (fill breaks password fields). */
export async function typeInto(locator: Locator, value: string) {
  await locator.click();
  await locator.fill("");
  await locator.pressSequentially(value, { delay: 10 });
  await expect(locator).toHaveValue(value);
}

export async function loginAdmin(
  page: Page,
  email: string,
  password: string,
  baseUrl = process.env.E2E_ADMIN_BASE_URL ?? "http://127.0.0.1:4322",
) {
  await page.goto(new URL("/login", baseUrl).toString());
  await typeInto(page.getByPlaceholder("Admin email"), email);
  const pwd = page.getByPlaceholder("Password");
  await typeInto(pwd, password);
  const loginResponse = page.waitForResponse(
    (r) => r.url().includes("/api/v1/auth/login") && r.request().method() === "POST",
  );
  await page.getByRole("button", { name: "Sign in" }).click();
  const resp = await loginResponse;
  expect(resp.ok(), `login HTTP ${resp.status()}`).toBeTruthy();
  await expect(page).not.toHaveURL(/\/login/);
}

export async function loginUser(page: Page, email: string, password: string) {
  await page.goto("/login");
  await typeInto(page.getByPlaceholder("Email"), email);
  const pwd = page.getByPlaceholder("Password");
  await typeInto(pwd, password);
  const loginResponse = page.waitForResponse(
    (r) => r.url().includes("/api/v1/auth/login") && r.request().method() === "POST",
  );
  await page.getByRole("button", { name: "Enter" }).click();
  const resp = await loginResponse;
  const safeBody = await resp.text().catch(() => "");
  const redacted = safeBody.replace(/"access_token"\s*:\s*"[^"]+"/g, '"access_token":"[redacted]"');
  expect(
    resp.ok(),
    `user login failed: POST ${resp.url()} HTTP ${resp.status()} body=${redacted.slice(0, 240)}`,
  ).toBeTruthy();
  await expect(page).not.toHaveURL(/\/login/);
}
