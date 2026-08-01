import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { uploadDocuments } from "../../lib/api";
import { ZipImportDialog } from "./ZipImportDialog";

vi.mock("../../lib/api", async (importOriginal) => {
  const original = await importOriginal<typeof import("../../lib/api")>();
  return { ...original, uploadDocuments: vi.fn() };
});

describe("ZipImportDialog", () => {
  it("explains how folders map to categories", () => {
    render(<ZipImportDialog categories={[]} onClose={vi.fn()} onImported={vi.fn()} open />);

    expect(screen.getByRole("dialog", { name: "Import documents" })).toBeVisible();
    fireEvent.click(screen.getByRole("tab", { name: "Folder ZIP" }));
    expect(screen.getByText("Folders become categories.")).toBeVisible();
    expect(screen.getByLabelText("Category for files at the ZIP root")).toHaveValue("Imported");
  });

  it("does not render while closed", () => {
    render(<ZipImportDialog categories={[]} onClose={vi.fn()} onImported={vi.fn()} open={false} />);

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("reports imported valid documents and invalid file signatures", async () => {
    vi.mocked(uploadDocuments).mockResolvedValue({ categoriesCreated: 0, documentsImported: 1, duplicatesSkipped: 0, unsupportedSkipped: 1, invalidSignatureSkipped: 2 });
    const { container } = render(<ZipImportDialog categories={[]} onClose={vi.fn()} onImported={vi.fn()} open />);
    const file = new File(["%PDF-1.7"], "policy.pdf", { type: "application/pdf" });

    fireEvent.change(container.querySelector('input[type="file"]')!, { target: { files: [file] } });
    fireEvent.click(screen.getByRole("button", { name: "Upload documents" }));

    expect(await screen.findByText("Valid documents were imported and preserved; all skips are reported below.")).toBeVisible();
    expect(screen.getByText("Invalid files").nextElementSibling).toHaveTextContent("2");
  });
});
