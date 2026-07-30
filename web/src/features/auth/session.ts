import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseQueryResult,
} from '@tanstack/react-query';

import { ApiError, api, type Bootstrap, type SetupInput, type SignInInput } from '@/lib/api';

/** Query key for the bootstrap document, the single source of session truth. */
export const BOOTSTRAP_KEY = ['bootstrap'] as const;

/**
 * Reads setup state and the current account.
 *
 * Polls while it is failing so the client reconnects on its own after the API
 * restarts, which happens constantly under cargo-watch.
 */
export function useBootstrap(): UseQueryResult<Bootstrap> {
  return useQuery({
    queryKey: BOOTSTRAP_KEY,
    queryFn: ({ signal }) => api.bootstrap(signal),
    // Session state changes only through actions this client takes, and those
    // invalidate the key explicitly.
    staleTime: 30_000,
    refetchInterval: (query) => (query.state.error === null ? false : 2_000),
    refetchOnWindowFocus: true,
  });
}

/** Creates the first administrator. */
export function useCompleteSetup() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: SetupInput) => api.completeSetup(input),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: BOOTSTRAP_KEY });
    },
  });
}

/** Signs in. */
export function useSignIn() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: SignInInput) => api.signIn(input),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: BOOTSTRAP_KEY });
    },
  });
}

/** Signs out. */
export function useSignOut() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => api.signOut(),
    onSuccess: async () => {
      // Everything cached was read as the signed-out account's predecessor, so
      // none of it should survive into the next session.
      queryClient.clear();
      await queryClient.invalidateQueries({ queryKey: BOOTSTRAP_KEY });
    },
  });
}

/**
 * Splits a failure into a field-level message and a form-level one.
 *
 * A validation error that names a field belongs next to that field; anything else
 * belongs in a summary above the form, where it cannot be missed.
 */
export function partitionError(error: unknown): {
  readonly formError: string | undefined;
  readonly fieldErrors: Readonly<Record<string, string>>;
} {
  if (error === null || error === undefined) {
    return { formError: undefined, fieldErrors: {} };
  }
  if (error instanceof ApiError && error.field !== undefined) {
    return { formError: undefined, fieldErrors: { [error.field]: error.message } };
  }
  if (error instanceof Error) {
    return { formError: error.message, fieldErrors: {} };
  }
  return { formError: 'Something went wrong. Please try again.', fieldErrors: {} };
}
