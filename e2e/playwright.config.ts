import { defineConfig, devices } from "@playwright/test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

// Do not load .env.user.local here — global-setup provisions a fresh user after config loads.
// Tests must call loadE2eUserCreds() so credentials match the current run.
const envLocal = path.join(__dirname, ".env.local");
for (const file of [envLocal].filter((p) => fs.existsSync(p))) {
  for (const line of fs.readFileSync(file, "utf8").split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const eq = trimmed.indexOf("=");
    if (eq <= 0) continue;
    const key = trimmed.slice(0, eq);
    const val = trimmed.slice(eq + 1);
    if (!process.env[key]) process.env[key] = val;
  }
}

const adminBase = process.env.E2E_ADMIN_BASE_URL ?? "http://127.0.0.1:4325";
const userBase = process.env.E2E_USER_BASE_URL ?? "http://127.0.0.1:4326";
const apiBase = process.env.E2E_API_BASE_URL ?? "http://127.0.0.1:4324";

const outputDir = path.join(os.tmpdir(), "pom-market-e2e-results");

export default defineConfig({
  globalSetup: require.resolve("./global-setup.ts"),
  testDir: "./tests",
  outputDir,
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: [["list"]],
  timeout: 120_000,
  use: {
    ...devices["Desktop Chrome"],
    trace: "on-first-retry",
  },
  projects: [
    { name: "tenant", testMatch: /tenant-api\.spec\.ts/, use: { baseURL: apiBase } },
    { name: "user-login", testMatch: /user-login\.spec\.ts/, use: { baseURL: userBase } },
    {
      name: "admin",
      testMatch: /admin-(login|kill-switch)\.spec\.ts/,
      use: { baseURL: adminBase },
    },
    { name: "paper", testMatch: /paper-flow\.spec\.ts/, use: { baseURL: userBase } },
    { name: "trading-data", testMatch: /trading-data\.spec\.ts/, use: { baseURL: userBase } },
  ],
});
