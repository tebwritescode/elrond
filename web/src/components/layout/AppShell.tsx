import { useState, type ReactNode } from "react";
import {
  Activity,
  Archive,
  BookOpen,
  Boxes,
  ChevronDown,
  CircleHelp,
  FileStack,
  FolderTree,
  LayoutDashboard,
  Search,
  Settings,
  Upload,
} from "lucide-react";

type AppShellProps = {
  children: ReactNode;
  connectionStatus: "connected" | "reconnecting";
  currentUsername?: string;
  onImport: () => void;
  onLogout: () => Promise<void>;
};

const navigation = [
  { label: "Overview", icon: LayoutDashboard, active: true },
  { label: "Library", icon: Archive },
  { label: "Categories", icon: FolderTree },
  { label: "Binders", icon: BookOpen },
  { label: "Activity", icon: Activity },
];

export function AppShell({ children, connectionStatus, currentUsername, onImport, onLogout }: AppShellProps) {
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
          {navigation.map(({ label, icon: Icon, active }) => (
            <button className={`nav-item${active ? " active" : ""}`} key={label} type="button">
              <Icon size={18} strokeWidth={1.8} aria-hidden="true" />
              <span>{label}</span>
              {label === "Library" && <span className="nav-count">0</span>}
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
          <div className="empty-tree">
            <Boxes size={18} strokeWidth={1.6} />
            <span>Your category tree will appear here.</span>
          </div>
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
            <input placeholder="Search documents, numbers, text..." type="search" />
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
