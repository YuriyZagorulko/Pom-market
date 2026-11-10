import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { FieldHelp } from "./field-help";
import { PrivateKeyHelp } from "./polymarket-credentials-hints";
import { POLYMARKET_ACCOUNT_LINKS } from "@/lib/polymarket-account-hints";

describe("FieldHelp", () => {
  it("opens and closes on trigger click", () => {
    render(
      <FieldHelp title="Test help">
        <p>Help body</p>
      </FieldHelp>,
    );
    const trigger = screen.getByRole("button", { name: "Help: Test help" });
    expect(screen.queryByText("Help body")).not.toBeInTheDocument();
    fireEvent.click(trigger);
    expect(screen.getByText("Help body")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByText("Help body")).not.toBeInTheDocument();
  });

  it("closes on Escape", () => {
    render(
      <FieldHelp title="Esc">
        <p>Panel</p>
      </FieldHelp>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Help: Esc" }));
    expect(screen.getByText("Panel")).toBeInTheDocument();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByText("Panel")).not.toBeInTheDocument();
  });
});

describe("PrivateKeyHelp", () => {
  it("shows Magic export link only for poly_proxy", () => {
    const { rerender } = render(<PrivateKeyHelp signatureType="poly_proxy" />);
    const link = screen.getByRole("link", { name: /reveal\.magic\.link/i });
    expect(link).toHaveAttribute("href", POLYMARKET_ACCOUNT_LINKS.magicKeyExport);
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");

    rerender(<PrivateKeyHelp signatureType="eoa" />);
    expect(screen.queryByRole("link", { name: /reveal\.magic\.link/i })).not.toBeInTheDocument();
    expect(screen.getByText(/Magic Link export page for EOA wallets/i)).toBeInTheDocument();
  });
});
