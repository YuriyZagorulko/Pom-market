import "@testing-library/jest-dom/vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DeleteStrategyModal } from "./delete-strategy-modal";

describe("DeleteStrategyModal", () => {
  it("requires typing DELETE before confirm is enabled", () => {
    const onConfirm = vi.fn();
    render(
      <DeleteStrategyModal
        open
        strategyName="Test strat"
        onConfirm={onConfirm}
        onCancel={vi.fn()}
      />,
    );
    const confirm = screen.getByTestId("confirmation-modal-confirm");
    expect(confirm).toBeDisabled();
    fireEvent.change(screen.getByTestId("delete-strategy-confirm-input"), {
      target: { value: "DELETE" },
    });
    expect(confirm).not.toBeDisabled();
    fireEvent.click(confirm);
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });
});
