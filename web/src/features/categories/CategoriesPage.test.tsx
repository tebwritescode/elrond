import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createCategory, deleteCategory, renameCategory } from "../../lib/api";
import { CategoriesPage } from "./CategoriesPage";

vi.mock("../../lib/api", async (importOriginal) => {
  const original = await importOriginal<typeof import("../../lib/api")>();
  return { ...original, createCategory: vi.fn(), deleteCategory: vi.fn(), renameCategory: vi.fn() };
});

const categories = [
  { id: "root", parentId: null, name: "Operations", documentCount: 2 },
  { id: "child", parentId: "root", name: "Safety", documentCount: 1 },
];

describe("CategoriesPage", () => {
  it("creates roots and children, renames, and confirms deletion", async () => {
    vi.mocked(createCategory).mockResolvedValue({ id: "new", parentId: null, name: "Records", documentCount: 0 });
    vi.mocked(renameCategory).mockResolvedValue();
    vi.mocked(deleteCategory).mockResolvedValue();
    const reload = vi.fn();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<CategoriesPage categories={categories} loading={false} onCatalogReload={reload} />);

    fireEvent.change(screen.getByLabelText("Create root category"), { target: { value: "Records" } });
    fireEvent.click(screen.getByRole("button", { name: "Create root" }));
    await waitFor(() => expect(createCategory).toHaveBeenCalledWith("Records", null));

    fireEvent.click(screen.getByRole("button", { name: "Add child to Operations" }));
    fireEvent.change(screen.getByLabelText("New child of Operations"), { target: { value: "Procedures" } });
    fireEvent.click(screen.getByRole("button", { name: "Add child" }));
    await waitFor(() => expect(createCategory).toHaveBeenCalledWith("Procedures", "root"));

    fireEvent.click(screen.getByRole("button", { name: "Rename Safety" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Rename Safety" }), { target: { value: "Workplace safety" } });
    fireEvent.click(screen.getByRole("button", { name: "Save Safety" }));
    await waitFor(() => expect(renameCategory).toHaveBeenCalledWith("child", "Workplace safety"));

    fireEvent.click(screen.getByRole("button", { name: "Delete Operations" }));
    await waitFor(() => expect(deleteCategory).toHaveBeenCalledWith("root"));
    expect(window.confirm).toHaveBeenCalledWith("Delete “Operations”? This cannot be undone.");
    expect(reload).toHaveBeenCalledTimes(4);
  });

  it("shows server conflict errors without flattening the tree", async () => {
    vi.mocked(deleteCategory).mockRejectedValue(new Error("Category contains documents."));
    vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<CategoriesPage categories={categories} loading={false} onCatalogReload={vi.fn()} />);

    fireEvent.click(screen.getByRole("button", { name: "Delete Operations" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Category contains documents.");
    expect(screen.getByText("Operations")).toBeVisible();
    expect(screen.getByText("Safety")).toBeVisible();
  });
});
