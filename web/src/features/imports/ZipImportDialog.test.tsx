import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ZipImportDialog } from "./ZipImportDialog";

describe("ZipImportDialog", () => {
  it("explains how folders map to categories", () => {
    render(<ZipImportDialog onClose={vi.fn()} onImported={vi.fn()} open />);

    expect(screen.getByRole("dialog", { name: "Import a folder hierarchy" })).toBeVisible();
    expect(screen.getByText("Folders become categories.")).toBeVisible();
    expect(screen.getByLabelText("Category for files at the ZIP root")).toHaveValue("Imported");
  });

  it("does not render while closed", () => {
    render(<ZipImportDialog onClose={vi.fn()} onImported={vi.fn()} open={false} />);

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
