import { Link, Outlet } from '@tanstack/react-router';

import {
  Button,
  IconAccounts,
  IconBinders,
  IconDashboard,
  IconDocuments,
  IconSignOut,
  ThemeToggle,
  Wordmark,
} from '@/components';
import { useBootstrap, useSignOut } from '@/features/auth/session';
import type { Role } from '@/lib/api';

/** One navigation entry. */
interface NavItem {
  readonly to: string;
  readonly label: string;
  readonly Icon: typeof IconDashboard;
  /** Minimum role needed to see the entry. */
  readonly requires?: Role;
}

const NAV: readonly NavItem[] = [
  { to: '/', label: 'Dashboard', Icon: IconDashboard },
  { to: '/documents', label: 'Documents', Icon: IconDocuments },
  { to: '/binders', label: 'Binders', Icon: IconBinders },
  { to: '/accounts', label: 'Accounts', Icon: IconAccounts, requires: 'admin' },
];

/** Role ladder, used to decide which navigation entries are visible. */
const ROLE_RANK: Readonly<Record<Role, number>> = {
  viewer: 0,
  reviewer: 1,
  editor: 2,
  admin: 3,
};

/** The persistent application frame: header, navigation, and content region. */
export function AppShell() {
  const bootstrap = useBootstrap();
  const signOut = useSignOut();
  const user = bootstrap.data?.user ?? null;
  const rank = user === null ? -1 : ROLE_RANK[user.role];

  return (
    <>
      {/*
        First focusable element on the page, so keyboard and screen reader users
        can reach the content without walking the whole navigation.
      */}
      <a className="el-skip-link" href="#main-content">
        Skip to main content
      </a>

      <div className="el-shell">
        <header className="el-shell__header">
          <Wordmark version={bootstrap.data?.version} />

          <div className="el-shell__spacer" />

          <ThemeToggle />

          {user !== null && (
            <div className="el-row">
              <span className="el-muted" style={{ fontSize: 'var(--el-text-sm)' }}>
                {user.username}
              </span>
              <Button
                variant="ghost"
                size="sm"
                icon={<IconSignOut size={15} />}
                isLoading={signOut.isPending}
                loadingLabel="Signing out"
                onClick={() => {
                  signOut.mutate();
                }}
              >
                Sign out
              </Button>
            </div>
          )}
        </header>

        <nav className="el-shell__sidebar" aria-label="Primary">
          <ul className="el-nav">
            {NAV.filter(
              (item) => item.requires === undefined || rank >= ROLE_RANK[item.requires],
            ).map(({ to, label, Icon }) => (
              <li key={to}>
                {/*
                    TanStack Router sets aria-current="page" on the active link,
                    which is what the stylesheet keys the active treatment off.
                  */}
                <Link to={to} className="el-nav__link" activeOptions={{ exact: to === '/' }}>
                  <Icon size={17} />
                  <span>{label}</span>
                </Link>
              </li>
            ))}
          </ul>
        </nav>

        <main className="el-shell__main" id="main-content" tabIndex={-1}>
          <Outlet />
        </main>
      </div>
    </>
  );
}
