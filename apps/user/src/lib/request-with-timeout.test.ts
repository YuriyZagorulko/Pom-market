import { describe, expect, it, vi } from "vitest";
import { ApiClient } from "@pom/api-client";
import { requestWithTimeout } from "./request-with-timeout";

describe("requestWithTimeout", () => {
  it("maps abort to a timeout message", async () => {
    const client = {
      request: vi.fn((_path: string, init?: RequestInit) =>
        new Promise((_resolve, reject) => {
          init?.signal?.addEventListener("abort", () => reject(new DOMException("Aborted", "AbortError")));
        }),
      ),
    } as unknown as ApiClient;
    await expect(
      requestWithTimeout(client, "/api/v1/source-traders", { method: "POST" }, 30),
    ).rejects.toThrow(/timed out/i);
  });
});
