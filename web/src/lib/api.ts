export type LibraryOverview = {
  setupRequired: boolean;
  documents: number;
  categories: number;
  binders: number;
  pendingReviews: number;
  stirlingConfigured: boolean;
};

export type SessionUser = {
  id: string;
  username: string;
  role: string;
};

export type ImportSummary = {
  categoriesCreated: number;
  documentsImported: number;
  duplicatesSkipped: number;
  unsupportedSkipped: number;
  invalidSignatureSkipped: number;
};

export type DocumentSummary = {
  id: string;
  title: string;
  status: "draft" | "in_review" | "published" | "archived";
  categoryId: string | null;
  categoryName: string | null;
  tags: string[];
  versionNumber: number;
  originalFilename: string;
  hasPdf: boolean;
  conversionStatus: "queued" | "processing" | "ready" | "failed";
  conversionError: string | null;
  updatedAt: string;
};

export type CategorySummary = {
  id: string;
  parentId: string | null;
  name: string;
  documentCount: number;
};

export async function fetchOverview(signal?: AbortSignal): Promise<LibraryOverview> {
  const response = await fetch("/api/v1/overview", {
    headers: { Accept: "application/json" },
    signal,
  });

  if (!response.ok) {
    throw new Error("The library overview is temporarily unavailable.");
  }

  return response.json() as Promise<LibraryOverview>;
}

export async function createInitialAdmin(username: string, password: string): Promise<void> {
  const response = await fetch("/api/v1/setup", {
    method: "POST",
    headers: {
      Accept: "application/json",
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ username, password }),
  });

  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { error?: string } | null;
    throw new Error(body?.error ?? "The administrator account could not be created.");
  }
}

export async function fetchCurrentUser(signal?: AbortSignal): Promise<SessionUser | null> {
  const response = await fetch("/api/v1/session", {
    headers: { Accept: "application/json" },
    signal,
  });
  if (response.status === 401) return null;
  if (!response.ok) throw new Error("The current session could not be checked.");
  return response.json() as Promise<SessionUser>;
}

export async function login(username: string, password: string): Promise<void> {
  const response = await fetch("/api/v1/session", {
    method: "POST",
    headers: {
      Accept: "application/json",
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ username, password }),
  });
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { error?: string } | null;
    throw new Error(body?.error ?? "Sign in failed.");
  }
}

export async function logout(): Promise<void> {
  const response = await fetch("/api/v1/session", { method: "DELETE" });
  if (!response.ok) throw new Error("Sign out failed.");
}

export async function importZipArchive(
  archive: File,
  rootCategory: string,
): Promise<ImportSummary> {
  const form = new FormData();
  form.append("archive", archive);
  form.append("rootCategory", rootCategory);
  const response = await fetch("/api/v1/imports/zip", {
    method: "POST",
    body: form,
  });
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { error?: string } | null;
    throw new Error(body?.error ?? "The ZIP archive could not be imported.");
  }
  return response.json() as Promise<ImportSummary>;
}

export async function uploadDocuments(
  files: File[],
  categoryPath: string[],
): Promise<ImportSummary> {
  const form = new FormData();
  files.forEach((file) => form.append("file", file));
  form.append("categoryPath", JSON.stringify(categoryPath));
  const response = await fetch("/api/v1/documents", {
    method: "POST",
    body: form,
  });
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { error?: string } | null;
    throw new Error(body?.error ?? "The document could not be uploaded.");
  }
  return response.json() as Promise<ImportSummary>;
}

export async function fetchDocuments(signal?: AbortSignal): Promise<DocumentSummary[]> {
  const response = await fetch("/api/v1/documents", {
    headers: { Accept: "application/json" },
    signal,
  });
  if (!response.ok) throw new Error("The document catalog could not be loaded.");
  return response.json() as Promise<DocumentSummary[]>;
}

export async function fetchCategories(signal?: AbortSignal): Promise<CategorySummary[]> {
  const response = await fetch("/api/v1/categories", {
    headers: { Accept: "application/json" },
    signal,
  });
  if (!response.ok) throw new Error("The category tree could not be loaded.");
  return response.json() as Promise<CategorySummary[]>;
}

async function mutationError(response: Response, fallback: string): Promise<Error> {
  const body = (await response.json().catch(() => null)) as { error?: string } | null;
  return new Error(body?.error ?? fallback);
}

export async function updateDocument(
  id: string,
  categoryId: string | null,
  tags: string[],
): Promise<void> {
  const response = await fetch(`/api/v1/documents/${encodeURIComponent(id)}`, {
    method: "PATCH",
    headers: { Accept: "application/json", "Content-Type": "application/json" },
    body: JSON.stringify({ categoryId, tags }),
  });
  if (!response.ok) throw await mutationError(response, "The document could not be updated.");
}

export async function createCategory(name: string, parentId: string | null): Promise<CategorySummary> {
  const response = await fetch("/api/v1/categories", {
    method: "POST",
    headers: { Accept: "application/json", "Content-Type": "application/json" },
    body: JSON.stringify({ name, parentId }),
  });
  if (!response.ok) throw await mutationError(response, "The category could not be created.");
  return response.json() as Promise<CategorySummary>;
}

export async function renameCategory(id: string, name: string): Promise<void> {
  const response = await fetch(`/api/v1/categories/${encodeURIComponent(id)}`, {
    method: "PATCH",
    headers: { Accept: "application/json", "Content-Type": "application/json" },
    body: JSON.stringify({ name }),
  });
  if (!response.ok) throw await mutationError(response, "The category could not be renamed.");
}

export async function deleteCategory(id: string): Promise<void> {
  const response = await fetch(`/api/v1/categories/${encodeURIComponent(id)}`, { method: "DELETE" });
  if (!response.ok) throw await mutationError(
    response,
    response.status === 409 ? "This category cannot be deleted while it contains documents or child categories." : "The category could not be deleted.",
  );
}

export async function downloadPrintableBinder(): Promise<void> {
  const response = await fetch("/api/v1/binders/printable.pdf", {
    headers: { Accept: "application/pdf" },
  });
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { error?: string } | null;
    throw new Error(body?.error ?? "The printable binder could not be generated.");
  }
  const blobUrl = URL.createObjectURL(await response.blob());
  const link = document.createElement("a");
  link.href = blobUrl;
  link.download = "elrond-library-binder.pdf";
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(blobUrl), 60_000);
}
