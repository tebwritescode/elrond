import { createRootRoute, createRoute, createRouter } from '@tanstack/react-router';

import { AccountsPage } from '@/features/admin/AccountsPage';
import { DashboardPage } from '@/features/dashboard/DashboardPage';
import { DocumentsPage } from '@/features/documents/DocumentsPage';
import { BindersPage } from '@/features/placeholders/ComingSoonPage';
import { AppShell } from '@/features/shell/AppShell';

import { NotFoundPage } from './NotFoundPage';

/**
 * The shell is the root route, so the header and navigation persist across
 * navigation instead of remounting.
 */
const rootRoute = createRootRoute({
  component: AppShell,
  notFoundComponent: () => (
    <NotFoundPage
      onGoHome={() => {
        void router.navigate({ to: '/' });
      }}
    />
  ),
});

const dashboardRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  component: DashboardPage,
});

const documentsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/documents',
  component: DocumentsPage,
});

const bindersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/binders',
  component: BindersPage,
});

/**
 * Authorization is enforced by the API, which returns 403 for a non-admin. The
 * route stays reachable so a mistyped link produces an explanation rather than a
 * silent redirect.
 */
const accountsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/accounts',
  component: AccountsPage,
});

const routeTree = rootRoute.addChildren([
  dashboardRoute,
  documentsRoute,
  bindersRoute,
  accountsRoute,
]);

export const router = createRouter({
  routeTree,
  defaultPreload: 'intent',
  // Deep links are served the shell by the Rust server's SPA fallback, so a
  // refresh on any route works.
  scrollRestoration: true,
});

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router;
  }
}
