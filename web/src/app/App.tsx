import { useEffect, useState } from "react";
import { AppShell } from "../components/layout/AppShell";
import { DashboardPage } from "../features/dashboard/DashboardPage";
import { fetchOverview, type LibraryOverview } from "../lib/api";

type LoadState =
  | { status: "loading" }
  | { status: "ready"; overview: LibraryOverview }
  | { status: "offline" };

export function App() {
  const [loadState, setLoadState] = useState<LoadState>({ status: "loading" });
  const [reloadKey, setReloadKey] = useState(0);

  useEffect(() => {
    const controller = new AbortController();
    let retryTimer: ReturnType<typeof setTimeout> | undefined;

    const loadOverview = () => {
      fetchOverview(controller.signal)
        .then((overview) => setLoadState({ status: "ready", overview }))
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

  return (
    <AppShell connectionStatus={loadState.status === "offline" ? "reconnecting" : "connected"}>
      <DashboardPage
        loadState={loadState}
        onSetupComplete={() => setReloadKey((key) => key + 1)}
      />
    </AppShell>
  );
}
