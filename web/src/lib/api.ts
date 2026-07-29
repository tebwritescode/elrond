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
