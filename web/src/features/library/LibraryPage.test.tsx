import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { updateDocument } from "../../lib/api";
import { LibraryPage } from "./LibraryPage";

vi.mock("../../lib/api", async (importOriginal) => {
  const original = await importOriginal<typeof import("../../lib/api")>();
  return { ...original, updateDocument: vi.fn() };
});

const document = {
  id: "doc/1",
  title: "Safety policy",
  status: "published" as const,
  categoryId: "category-1",
  categoryName: "Policies",
  tags: ["safety", "annual"],
  versionNumber: 2,
  originalFilename: "safety.docx",
  hasPdf: true,
  conversionStatus: "ready" as const,
  conversionError: null,
  updatedAt: "2026-07-31T00:00:00Z",
};

describe("LibraryPage", () => {
  it("opens the same-origin PDF and exposes the original attachment route", () => {
    const open = vi.spyOn(window, "open").mockImplementation(() => null);
    render(<LibraryPage categories={[]} documents={[document]} loading={false} onCatalogReload={vi.fn()} onQueryChange={vi.fn()} query="" />);

    fireEvent.click(screen.getByText("Safety policy"));
    fireEvent.click(screen.getByRole("button", { name: "Open document" }));

    expect(open).toHaveBeenCalledWith("/api/v1/documents/doc%2F1/pdf", "_blank", "noopener,noreferrer");
    expect(screen.getByRole("link", { name: "Download original" })).toHaveAttribute("href", "/api/v1/documents/doc%2F1/original");
    open.mockRestore();
  });

  it("saves the primary category and normalized tags, then reloads the catalog", async () => {
    vi.mocked(updateDocument).mockResolvedValue();
    const reload = vi.fn();
    render(<LibraryPage categories={[{ id: "category-2", parentId: null, name: "Manuals", documentCount: 0 }]} documents={[document]} loading={false} onCatalogReload={reload} onQueryChange={vi.fn()} query="" />);

    fireEvent.click(screen.getByText("Safety policy"));
    fireEvent.change(screen.getByLabelText("Primary category"), { target: { value: "category-2" } });
    fireEvent.change(screen.getByLabelText("Tags"), { target: { value: "manual, current, manual" } });
    fireEvent.click(screen.getByRole("button", { name: "Save details" }));

    await waitFor(() => expect(updateDocument).toHaveBeenCalledWith("doc/1", "category-2", ["manual", "current"]));
    expect(await screen.findByText("Document details saved.")).toBeVisible();
    expect(reload).toHaveBeenCalledOnce();
  });

  it("includes tags in filtering and displays them", () => {
    render(<LibraryPage categories={[]} documents={[document]} loading={false} onCatalogReload={vi.fn()} onQueryChange={vi.fn()} query="annual" />);
    expect(screen.getByText("Safety policy")).toBeVisible();
    expect(screen.getByText("annual")).toBeVisible();
  });
});
