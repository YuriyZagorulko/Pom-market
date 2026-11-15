import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ApiClient } from "@pom/api-client";
import { StrategyDangerZone } from "./strategy-danger-zone";
import { STRATEGY_HARD_CLOSE_API_CONFIRMATION } from "./strategy-hard-close-copy";

vi.mock("@/components/toast", () => ({
  useToast: () => ({ error: vi.fn(), success: vi.fn(), info: vi.fn() }),
}));

afterEach(() => cleanup());

const control = {
  trading_control_state: "normal",
  blocks_buy: false,
  blocks_all_trading: false,
  allows_copy_sell: true,
  resume_requires_confirmation: false,
};

function mockClient(hardClosePost = vi.fn()): ApiClient {
  return {
    request: vi.fn(async (path: string, init?: RequestInit) => {
      if (path.endsWith("/trading-control") && (!init || init.method === undefined)) {
        return control;
      }
      if (path.endsWith("/hard-close") && init?.method === "POST") {
        return hardClosePost();
      }
      throw new Error(`unexpected ${path}`);
    }),
  } as unknown as ApiClient;
}

describe("StrategyDangerZone hard close", () => {
  it("opens modal without phrase input; Cancel skips POST", async () => {
    const post = vi.fn().mockResolvedValue({ operation_id: "op-1" });
    const client = mockClient(post);
    render(<StrategyDangerZone strategyId="str-1" client={client} />);
    await waitFor(() => expect(screen.getByTestId("strategy-hard-close-open")).toBeEnabled());
    expect(screen.queryByPlaceholderText(/CLOSE STRATEGY/i)).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId("strategy-hard-close-open"));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByText(/Manual positions and other strategies/i)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(post).not.toHaveBeenCalled();
  });

  it("Confirm sends API confirmation once", async () => {
    const post = vi.fn().mockResolvedValue({ operation_id: "op-2" });
    const client = mockClient(post);
    render(<StrategyDangerZone strategyId="str-1" client={client} />);
    await waitFor(() => expect(screen.getByTestId("strategy-hard-close-open")).toBeEnabled());

    fireEvent.click(screen.getByTestId("strategy-hard-close-open"));
    fireEvent.click(screen.getByTestId("confirmation-modal-confirm"));

    await waitFor(() => expect(post).toHaveBeenCalledTimes(1));
    const call = (client.request as ReturnType<typeof vi.fn>).mock.calls.find(
      ([path, init]) => path.endsWith("/hard-close") && init?.method === "POST",
    );
    expect(call).toBeDefined();
    expect(JSON.parse(call![1].body as string).confirmation).toBe(STRATEGY_HARD_CLOSE_API_CONFIRMATION);
  });
});
