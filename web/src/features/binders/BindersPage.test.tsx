import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { BindersPage } from "./BindersPage";

describe("BindersPage", () => {
  it("explains and offers printable binder generation", () => {
    render(<BindersPage />);

    expect(screen.getByText(/includes every latest PDF-ready document/)).toBeVisible();
    expect(screen.getByText(/includes category and document separator pages/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Generate printable binder" })).toBeEnabled();
  });
});
