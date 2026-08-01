import { useEffect, useState } from "react";
import { AppShell, type WorkspaceSection } from "../components/layout/AppShell";
import { LoginPage } from "../features/auth/LoginPage";
import { BindersPage } from "../features/binders/BindersPage";
import { CategoriesPage } from "../features/categories/CategoriesPage";
import { DashboardPage } from "../features/dashboard/DashboardPage";
import { ZipImportDialog } from "../features/imports/ZipImportDialog";
import { LibraryPage } from "../features/library/LibraryPage";
import {
  fetchCategories,
  fetchCurrentUser,
  fetchDocuments,
  fetchOverview,
  logout,
  type CategorySummary,
  type DocumentSummary,
  type LibraryOverview,
  type SessionUser,
} from "../lib/api";

type LoadState =
  | { status: "loading" }
  | { status: "ready"; overview: LibraryOverview }
  | { status: "offline" };

export function App() {
  const [loadState, setLoadState] = useState<LoadState>({ status: "loading" });
  const [currentUser, setCurrentUser] = useState<SessionUser | null>();
  const [reloadKey, setReloadKey] = useState(0);
  const [catalogReloadKey, setCatalogReloadKey] = useState(0);
  const [importOpen, setImportOpen] = useState(false);
  const [activeSection, setActiveSection] = useState<WorkspaceSection>("overview");
  const [query, setQuery] = useState("");
  const [documents, setDocuments] = useState<DocumentSummary[]>([]);
  const [categories, setCategories] = useState<CategorySummary[]>([]);
  const [catalogLoading, setCatalogLoading] = useState(false);

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

  useEffect(() => {
    if (!currentUser) return;
    const controller = new AbortController();
    setCatalogLoading(true);
    Promise.all([fetchDocuments(controller.signal), fetchCategories(controller.signal)])
      .then(([loadedDocuments, loadedCategories]) => {
        setDocuments(loadedDocuments);
        setCategories(loadedCategories);
      })
      .catch((error: unknown) => {
        if (!(error instanceof DOMException && error.name === "AbortError")) {
          setDocuments([]);
          setCategories([]);
        }
      })
      .finally(() => setCatalogLoading(false));
    return () => controller.abort();
  }, [currentUser, reloadKey, catalogReloadKey]);

  useEffect(() => {
    if (loadState.status !== "ready" || !loadState.overview.stirlingConfigured || !currentUser || !documents.some((document) => ["queued", "processing"].includes(document.conversionStatus))) return;
    const controller = new AbortController();
    const timer = setInterval(() => {
      fetchDocuments(controller.signal)
        .then(setDocuments)
        .catch((error: unknown) => {
          if (!(error instanceof DOMException && error.name === "AbortError")) console.error(error);
        });
    }, 3_000);
    return () => {
      controller.abort();
      clearInterval(timer);
    };
  }, [currentUser, documents, loadState]);

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
      activeSection={activeSection}
      categories={categories}
      documentCount={documents.length}
      onNavigate={setActiveSection}
      onQueryChange={setQuery}
      query={query}
      onImport={() => setImportOpen(true)}
      onLogout={async () => {
        await logout();
        setCurrentUser(null);
      }}
    >
      {activeSection === "overview" && (
        <DashboardPage
          loadState={loadState}
          onImport={() => setImportOpen(true)}
          onSetupComplete={() => setReloadKey((key) => key + 1)}
        />
      )}
      {activeSection === "library" && (
        <LibraryPage categories={categories} documents={documents} loading={catalogLoading} onCatalogReload={() => setCatalogReloadKey((key) => key + 1)} onQueryChange={setQuery} query={query} />
      )}
      {activeSection === "categories" && <CategoriesPage categories={categories} loading={catalogLoading} onCatalogReload={() => setCatalogReloadKey((key) => key + 1)} />}
      {activeSection === "binders" && <BindersPage />}
      {activeSection === "activity" && (
        <section className="planned-workspace">
          <p className="eyebrow">Next workspace</p>
          <h1>Activity log</h1>
          <p>Uploads, approvals, and releases will be presented as a permanent audit timeline.</p>
        </section>
      )}
      <ZipImportDialog
        categories={categories}
        onClose={() => setImportOpen(false)}
        onImported={() => setReloadKey((key) => key + 1)}
        open={importOpen}
      />
    </AppShell>
  );
}
