import { useState, type ReactNode } from "react";
import {
  Activity,
  Archive,
  BookOpen,
  ChevronDown,
  CircleHelp,
  FileStack,
  FolderTree,
  LayoutDashboard,
  Search,
  Settings,
  Upload,
} from "lucide-react";
import type { CategorySummary } from "../../lib/api";

export type WorkspaceSection = "overview" | "library" | "categories" | "binders" | "activity";

type AppShellProps = {
  children: ReactNode;
  connectionStatus: "connected" | "reconnecting";
  currentUsername?: string;
  activeSection: WorkspaceSection;
  categories: CategorySummary[];
  documentCount: number;
  query: string;
  onNavigate: (section: WorkspaceSection) => void;
  onQueryChange: (query: string) => void;
  onImport: () => void;
  onLogout: () => Promise<void>;
};

const navigation = [
  { id: "overview", label: "Overview", icon: LayoutDashboard },
  { id: "library", label: "Library", icon: Archive },
  { id: "categories", label: "Categories", icon: FolderTree },
  { id: "binders", label: "Binders", icon: BookOpen },
  { id: "activity", label: "Activity", icon: Activity },
] satisfies Array<{ id: WorkspaceSection; label: string; icon: typeof LayoutDashboard }>;

export function AppShell({
  children,
  connectionStatus,
  currentUsername,
  activeSection,
  categories,
  documentCount,
  query,
  onNavigate,
  onQueryChange,
  onImport,
  onLogout,
}: AppShellProps) {
  const [accountOpen, setAccountOpen] = useState(false);
  const initials = currentUsername?.slice(0, 2).toUpperCase() ?? "EL";
  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="wordmark" aria-label="Elrond home">
          <div className="wordmark-mark" aria-hidden="true">
            <FileStack size={20} strokeWidth={1.8} />
          </div>
          <div>
            <strong>ELROND</strong>
            <span>DOCUMENT LIBRARY</span>
          </div>
        </div>

        <nav className="primary-nav" aria-label="Main navigation">
          <p className="nav-label">Workspace</p>
          {navigation.map(({ id, label, icon: Icon }) => (
            <button
              className={`nav-item${activeSection === id ? " active" : ""}`}
              key={id}
              onClick={() => onNavigate(id)}
              type="button"
            >
              <Icon size={18} strokeWidth={1.8} aria-hidden="true" />
              <span>{label}</span>
              {id === "library" && <span className="nav-count">{documentCount}</span>}
            </button>
          ))}
        </nav>

        <div className="sidebar-collection">
          <div className="collection-heading">
            <span>Categories</span>
            <button type="button" aria-label="Category options">
              <ChevronDown size={14} />
            </button>
          </div>
          {categories.length === 0 ? (
            <div className="empty-tree"><FolderTree size={18} strokeWidth={1.6} /><span>Your category tree will appear here.</span></div>
          ) : (
            <div className="sidebar-tree">
              {categories.filter((category) => category.parentId === null).slice(0, 7).map((category) => (
                <button key={category.id} onClick={() => onNavigate("categories")} type="button">
                  <FolderTree size={14} /><span>{category.name}</span><small>{category.documentCount}</small>
                </button>
              ))}
            </div>
          )}
        </div>

        <div className="sidebar-footer">
          <button className="nav-item" type="button">
            <Settings size={18} strokeWidth={1.8} />
            <span>Settings</span>
          </button>
          <button className="nav-item" type="button">
            <CircleHelp size={18} strokeWidth={1.8} />
            <span>Help & shortcuts</span>
          </button>
          <div className={`connection-state ${connectionStatus}`}>
            <span aria-hidden="true" />
            {connectionStatus === "connected" ? "Local library online" : "Reconnecting to library"}
          </div>
        </div>
      </aside>

      <div className="workspace">
        <header className="topbar">
          <label className="global-search">
            <Search size={18} strokeWidth={1.8} aria-hidden="true" />
            <span className="sr-only">Search the library</span>
            <input
              onChange={(event) => {
                onQueryChange(event.target.value);
                if (event.target.value) onNavigate("library");
              }}
              placeholder="Search documents, numbers, text..."
              type="search"
              value={query}
            />
            <kbd>Ctrl K</kbd>
          </label>
          <button className="upload-button" disabled={!currentUsername} onClick={onImport} type="button">
            <Upload size={17} strokeWidth={2} aria-hidden="true" />
            Import documents
          </button>
          <button
            aria-expanded={accountOpen}
            aria-label="Open account menu"
            className="profile-button"
            onClick={() => setAccountOpen((open) => !open)}
            type="button"
          >
            <span>{initials}</span>
            <ChevronDown size={14} />
          </button>
          {accountOpen && (
            <div className="account-menu">
              <div><small>Signed in as</small><strong>{currentUsername}</strong></div>
              <button onClick={() => void onLogout()} type="button">Sign out</button>
            </div>
          )}
        </header>
        <main>{children}</main>
      </div>
    </div>
  );
}
