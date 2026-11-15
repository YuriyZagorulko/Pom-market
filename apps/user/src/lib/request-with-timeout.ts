import { ApiClient } from "@pom/api-client";

const ADD_WALLET_TIMEOUT_MS = 45_000;
export const DELETE_STRATEGY_TIMEOUT_MS = 120_000;
export const SOURCE_TRADER_CALCULATE_TIMEOUT_MS = 180_000;

export async function requestWithTimeout<T>(
  client: ApiClient,
  path: string,
  init: RequestInit,
  timeoutMs = ADD_WALLET_TIMEOUT_MS,
  timeoutHint?: string,
): Promise<T> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await client.request<T>(path, { ...init, signal: controller.signal });
  } catch (err) {
    if (err instanceof DOMException && err.name === "AbortError") {
      throw new Error(
        timeoutHint ??
          `Request timed out after ${Math.round(timeoutMs / 1000)}s. The wallet may still have been added — check Source traders before retrying.`,
      );
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }
}
