import { useEffect, useState } from "react";
import { AppShell } from "../components/layout/AppShell";
import { LoginPage } from "../features/auth/LoginPage";
import { DashboardPage } from "../features/dashboard/DashboardPage";
import { ZipImportDialog } from "../features/imports/ZipImportDialog";
import { fetchCurrentUser, fetchOverview, logout, type LibraryOverview, type SessionUser } from "../lib/api";

type LoadState =
  | { status: "loading" }
  | { status: "ready"; overview: LibraryOverview }
  | { status: "offline" };

export function App() {
  const [loadState, setLoadState] = useState<LoadState>({ status: "loading" });
  const [currentUser, setCurrentUser] = useState<SessionUser | null>();
  const [reloadKey, setReloadKey] = useState(0);
  const [importOpen, setImportOpen] = useState(false);

  useEffect(() => {
    const controller = new AbortController();
    let retryTimer: ReturnType<typeof setTimeout> | undefined;

    const loadOverview = () => {
      Promise.all([fetchOverview(controller.signal), fetchCurrentUser(controller.signal)])
        .then(([overview, user]) => {
          setLoadState({ status: "ready", overview });
          setCurrentUser(user);
        })
        .catch((error: unknown) => {
          if (error instanceof DOMException && error.name === "AbortError") return;
          setLoadState({ status: "offline" });
          retryTimer = setTimeout(loadOverview, 2_000);
        });
    };

    loadOverview();

    return () => {
      controller.abort();
      if (retryTimer) clearTimeout(retryTimer);
    };
  }, [reloadKey]);

  if (
    loadState.status === "ready" &&
    !loadState.overview.setupRequired &&
    currentUser === null
  ) {
    return <LoginPage onLogin={() => setReloadKey((key) => key + 1)} />;
  }

  return (
    <AppShell
      connectionStatus={loadState.status === "offline" ? "reconnecting" : "connected"}
      currentUsername={currentUser?.username}
      onImport={() => setImportOpen(true)}
      onLogout={async () => {
        await logout();
        setCurrentUser(null);
      }}
    >
      <DashboardPage
        loadState={loadState}
        onImport={() => setImportOpen(true)}
        onSetupComplete={() => setReloadKey((key) => key + 1)}
      />
      <ZipImportDialog
        onClose={() => setImportOpen(false)}
        onImported={() => setReloadKey((key) => key + 1)}
        open={importOpen}
      />
    </AppShell>
  );
}
