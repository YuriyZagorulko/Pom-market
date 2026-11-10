import { execSync } from "node:child_process";
import * as path from "node:path";

const repoRoot = path.resolve(__dirname, "../..");

/** Injects a mock source trade via backend integration test (requires host `cargo`). */
export function injectPaperTrade(wallet: string, side: string, txSuffix: string) {
  const workerUrl = process.env.WORKER_DATABASE_URL;
  if (!workerUrl?.includes("pommarket_e2e")) {
    throw new Error("WORKER_DATABASE_URL must target pommarket_e2e for inject");
  }
  execSync(
    `INJECT_WALLET=${wallet} INJECT_SIDE=${side} INJECT_TX_SUFFIX=${txSuffix} bash scripts/inject-paper-event.sh`,
    {
      cwd: repoRoot,
      stdio: "pipe",
      env: {
        ...process.env,
        E2E_MODE: "1",
        WORKER_DATABASE_URL: workerUrl,
      },
    },
  );
}
