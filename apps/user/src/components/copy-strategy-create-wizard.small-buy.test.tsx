import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { CopyStrategyCreateWizard } from "./copy-strategy-create-wizard";
import { SMALL_BUY_OVERRIDE_DESCRIPTION } from "@/lib/strategy-settings-small-buy";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: vi.fn() }),
}));
vi.mock("@/lib/session", () => ({
  useSession: () => ({ client: null, status: "unauthenticated" }),
}));
vi.mock("@/components/desk", () => ({
  useResource: () => ({ data: null, loading: false, error: null }),
}));
vi.mock("@/components/toast", () => ({
  useToast: () => ({ success: vi.fn(), error: vi.fn() }),
}));

describe("CopyStrategyCreateWizard Small BUY copy", () => {
  it("shows shared at-or-below description for PVR and Equal Copy", () => {
    render(<CopyStrategyCreateWizard backHref="/" backLabel="Back" />);
    expect(screen.getByTestId("small-buy-override-description")).toHaveTextContent(
      SMALL_BUY_OVERRIDE_DESCRIPTION,
    );
    const sizingSelect = screen
      .getAllByRole("combobox")
      .find((el) => (el as HTMLSelectElement).value === "position_value_ratio")!;
    fireEvent.change(sizingSelect, { target: { value: "proportional" } });
    expect(screen.getByTestId("small-buy-override-description")).toBeInTheDocument();
  });

  it("shows fixed mode small BUY note", () => {
    render(<CopyStrategyCreateWizard backHref="/" backLabel="Back" />);
    const sizingSelect = screen
      .getAllByRole("combobox")
      .find((el) => (el as HTMLSelectElement).value === "position_value_ratio")!;
    fireEvent.change(sizingSelect, { target: { value: "fixed_amount" } });
    expect(screen.getByTestId("fixed-small-buy-note")).toHaveTextContent(/1×/i);
    expect(screen.getByTestId("fixed-small-buy-note")).toHaveTextContent(/bypassed/i);
  });

  it("shows multiplier hint when override enabled on Equal Copy", () => {
    render(<CopyStrategyCreateWizard backHref="/" backLabel="Back" />);
    const sizingSelect = screen
      .getAllByRole("combobox")
      .find((el) => (el as HTMLSelectElement).value === "position_value_ratio")!;
    fireEvent.change(sizingSelect, { target: { value: "proportional" } });
    const checkboxes = screen.getAllByRole("checkbox");
    const override = checkboxes.find(
      (el) => el.closest("label")?.textContent?.includes("Enable Small BUY Override"),
    );
    expect(override).toBeDefined();
    fireEvent.click(override!);
    expect(screen.getByTestId("small-buy-override-multiplier-hint")).toHaveTextContent(/Copy %/i);
  });
});