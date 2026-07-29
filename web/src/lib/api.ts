export type LibraryOverview = {
  setupRequired: boolean;
  documents: number;
  categories: number;
  binders: number;
  pendingReviews: number;
  stirlingConfigured: boolean;
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
