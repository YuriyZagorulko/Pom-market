import "@testing-library/jest-dom/vitest";
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { SIZING_MODE_OPTIONS } from "@/lib/sizing-mode";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: vi.fn() }),
}));

vi.mock("@/lib/session", () => ({
  useSession: () => ({ client: { request: vi.fn() } }),
}));

vi.mock("@/components/toast", () => ({
  useToast: () => ({ error: vi.fn(), success: vi.fn(), info: vi.fn() }),
}));

vi.mock("@/components/desk", () => ({
  useResource: (path: string) => {
    if (path.includes("allocation-summary")) {
      return {
        data: {
          account_equity_usd: "5000",
          unallocated_usd: "5000",
          strategies: [],
        },
        loading: false,
        error: null,
      };
    }
    if (path.includes("source-traders/") && !path.endsWith("/source-traders")) {
      return {
        data: { id: "00000000-0000-0000-0000-000000000001", monitoring_enabled: true, alias: "T" },
        loading: false,
        error: null,
      };
    }
    return { data: [], loading: false, error: null };
  },
}));

import { CopyStrategyCreateWizard } from "./copy-strategy-create-wizard";

describe("CopyStrategyCreateWizard sizing modes", () => {
  it("offers exactly three user sizing modes", () => {
    expect(SIZING_MODE_OPTIONS).toHaveLength(3);
    render(<CopyStrategyCreateWizard fixedTraderId="00000000-0000-0000-0000-000000000001" />);
    const allowed = new Set(["position_value_ratio", "fixed_amount", "proportional"]);
    const optionEls = screen.getAllByRole("option");
    const sizingValues = optionEls
      .map((o) => (o as HTMLOptionElement).value)
      .filter((v) => allowed.has(v));
    expect(new Set(sizingValues).size).toBe(3);
    expect(sizingValues).toHaveLength(3);
    expect(screen.queryByRole("option", { name: /Portfolio-relative/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("option", { name: /Multiplier/i })).not.toBeInTheDocument();
  });
});
