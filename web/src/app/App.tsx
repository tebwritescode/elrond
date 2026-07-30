import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';

import { Button, Callout, Spinner, Wordmark } from '@/components';
import { useBootstrap } from '@/features/auth/session';
import { SetupPage } from '@/features/auth/SetupPage';
import { SignInPage } from '@/features/auth/SignInPage';
import { ApiError, NetworkError } from '@/lib/api';

import { router } from './router';
import { ThemeProvider } from './ThemeProvider';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      /*
       * Retry only what retrying can fix. A 4xx is a decision the server has
       * already made, so repeating the request just delays the error the user
       * needs to see; a connectivity failure is worth a few attempts.
       */
      retry: (failureCount, error) => {
        if (error instanceof ApiError) {
          return error.status >= 500 && failureCount < 2;
        }
        return error instanceof NetworkError && failureCount < 3;
      },
      refetchOnWindowFocus: false,
    },
    mutations: {
      // A mutation has a side effect; retrying it automatically risks performing
      // that side effect twice.
      retry: false,
    },
  },
});

/** Application root: providers, then the gate that picks the first screen. */
export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <Gate />
      </ThemeProvider>
    </QueryClientProvider>
  );
}

/**
 * Chooses between setup, sign-in, and the workspace.
 *
 * Implemented as a gate rather than as route guards so the current URL survives
 * signing in: a deep link visited while signed out lands on that page once
 * authentication succeeds, with no redirect bookkeeping.
 */
function Gate() {
  const bootstrap = useBootstrap();

  if (bootstrap.isPending) {
    return <FullPageStatus>Loading Elrond…</FullPageStatus>;
  }

  if (bootstrap.isError) {
    return (
      <UnreachableServer error={bootstrap.error} onRetry={() => void bootstrap.refetch()} />
    );
  }

  const { requires_setup: requiresSetup, user, version } = bootstrap.data;

  if (requiresSetup) {
    return <SetupPage version={version} />;
  }
  if (user === null) {
    return <SignInPage version={version} />;
  }
  return <RouterProvider router={router} />;
}

/** Centred status message used before the shell can render. */
function FullPageStatus({ children }: { readonly children: string }) {
  return (
    <main className="el-gate">
      <div className="el-row" role="status">
        <Spinner />
        <span className="el-muted">{children}</span>
      </div>
    </main>
  );
}

/**
 * Shown when the API cannot be reached.
 *
 * The bootstrap query polls while it is failing, so this screen clears itself as
 * soon as the server is back. That is the normal state for a second or two every
 * time cargo-watch rebuilds, which is why it explains itself rather than just
 * reporting an error.
 */
function UnreachableServer({
  error,
  onRetry,
}: {
  readonly error: Error;
  readonly onRetry: () => void;
}) {
  const isOffline = error instanceof NetworkError;

  return (
    <main className="el-gate">
      <div className="el-gate__card">
        <Wordmark />
        <h1 style={{ marginTop: 'var(--el-space-4)' }}>
          {isOffline ? 'Waiting for the server' : 'The server returned an error'}
        </h1>
        <div style={{ marginTop: 'var(--el-space-4)' }}>
          <Callout tone={isOffline ? 'caution' : 'danger'} title={error.name}>
            {error.message}
            {isOffline && ' Retrying automatically.'}
          </Callout>
        </div>
        <div className="el-row" style={{ marginTop: 'var(--el-space-5)' }}>
          <Button variant="primary" onClick={onRetry}>
            Try again now
          </Button>
        </div>
      </div>
    </main>
  );
}
