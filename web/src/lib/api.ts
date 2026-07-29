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
