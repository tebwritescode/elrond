import { afterEach, describe, expect, it, vi } from "vitest";
import { createCategory, deleteCategory, renameCategory, updateDocument } from "./api";

afterEach(() => vi.unstubAllGlobals());

describe("mutation API helpers", () => {
  it("patches document metadata with the required body", async () => {
    const fetch = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetch);

    await updateDocument("doc/1", null, ["approved"]);

    expect(fetch).toHaveBeenCalledWith("/api/v1/documents/doc%2F1", expect.objectContaining({
      method: "PATCH",
      body: JSON.stringify({ categoryId: null, tags: ["approved"] }),
    }));
  });

  it("uses category collection and item routes", async () => {
    const category = { id: "new", parentId: "parent", name: "Child", documentCount: 0 };
    const fetch = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify(category), { status: 201, headers: { "Content-Type": "application/json" } }))
      .mockResolvedValueOnce(new Response(null, { status: 204 }))
      .mockResolvedValueOnce(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetch);

    await expect(createCategory("Child", "parent")).resolves.toEqual(category);
    await renameCategory("new", "Renamed");
    await deleteCategory("new");

    expect(fetch).toHaveBeenNthCalledWith(1, "/api/v1/categories", expect.objectContaining({ method: "POST", body: JSON.stringify({ name: "Child", parentId: "parent" }) }));
    expect(fetch).toHaveBeenNthCalledWith(2, "/api/v1/categories/new", expect.objectContaining({ method: "PATCH", body: JSON.stringify({ name: "Renamed" }) }));
    expect(fetch).toHaveBeenNthCalledWith(3, "/api/v1/categories/new", { method: "DELETE" });
  });

  it("surfaces category conflict details from the server", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ error: "Category contains documents." }), { status: 409, headers: { "Content-Type": "application/json" } })));
    await expect(deleteCategory("occupied")).rejects.toThrow("Category contains documents.");
  });
});
