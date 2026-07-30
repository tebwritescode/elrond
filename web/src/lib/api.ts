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
};

export type DocumentSummary = {
  id: string;
  title: string;
  status: "draft" | "in_review" | "published" | "archived";
  categoryName: string | null;
  versionNumber: number;
  originalFilename: string;
  hasPdf: boolean;
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

export async function uploadDocument(
  file: File,
  categoryPath: string[],
): Promise<ImportSummary> {
  const form = new FormData();
  form.append("file", file);
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
