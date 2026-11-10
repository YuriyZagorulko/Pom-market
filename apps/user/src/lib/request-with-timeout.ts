import { ApiClient } from "@pom/api-client";

const ADD_WALLET_TIMEOUT_MS = 45_000;

export async function requestWithTimeout<T>(
  client: ApiClient,
  path: string,
  init: RequestInit,
  timeoutMs = ADD_WALLET_TIMEOUT_MS,
): Promise<T> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await client.request<T>(path, { ...init, signal: controller.signal });
  } catch (err) {
    if (err instanceof DOMException && err.name === "AbortError") {
      throw new Error(
        `Request timed out after ${Math.round(timeoutMs / 1000)}s. The wallet may still have been added — check Source traders before retrying.`,
      );
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }
}
