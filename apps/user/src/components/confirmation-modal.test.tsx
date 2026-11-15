import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ConfirmationModal } from "./confirmation-modal";

describe("ConfirmationModal", () => {
  it("Cancel calls onCancel without confirm", () => {
    const onCancel = vi.fn();
    const onConfirm = vi.fn();
    render(
      <ConfirmationModal
        open
        title="Test"
        description="Body"
        confirmLabel="Confirm"
        onConfirm={onConfirm}
        onCancel={onCancel}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("Confirm calls onConfirm once", () => {
    const onConfirm = vi.fn();
    render(
      <ConfirmationModal
        open
        title="Test"
        description="Body"
        confirmLabel="Go"
        onConfirm={onConfirm}
        onCancel={() => undefined}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Go" }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it("does not cancel on Escape while busy", () => {
    const onCancel = vi.fn();
    render(
      <ConfirmationModal
        open
        title="Test"
        description="Body"
        confirmLabel="Go"
        busy
        onConfirm={() => undefined}
        onCancel={onCancel}
      />,
    );
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onCancel).not.toHaveBeenCalled();
  });
});
