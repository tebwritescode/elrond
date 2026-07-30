/**
 * Typed client for the Elrond API.
 *
 * Every response shape here mirrors a Rust DTO in `crates/api/src/routes`. The
 * two are kept in step by hand for now; a generated contract is worth adding once
 * the surface stops changing shape every milestone.
 */

/** Base path for every endpoint. Matches `API_PREFIX` in the Rust router. */
const API_BASE = '/api/v1';

/** Header the server expects the CSRF token echoed in. */
const CSRF_HEADER = 'X-Elrond-CSRF';

/** Authority levels, ordered least to most privileged. */
export type Role = 'viewer' | 'reviewer' | 'editor' | 'admin';

/**
 * An account, as the API renders it.
 *
 * There is no email address or other contact detail in the model: authentication
 * is a username and a password, and nothing else is stored.
 */
export interface UserView {
  readonly id: string;
  readonly username: string;
  readonly role: Role;
  readonly is_active: boolean;
  readonly created_at: string;
}

/** Everything the shell needs to choose its first screen. */
export interface Bootstrap {
  readonly requires_setup: boolean;
  readonly user: UserView | null;
  readonly csrf_token: string;
  readonly version: string;
}

/** Result of establishing a session. */
export interface SessionCreated {
  readonly user: UserView;
  readonly csrf_token: string;
  readonly expires_at: string;
}

/** The JSON envelope every API failure uses. */
interface ErrorBody {
  readonly code: string;
  readonly message: string;
  readonly field?: string;
}

/**
 * A failure the server reported.
 *
 * `code` is the stable identifier to branch on; `message` is safe to display.
 */
export class ApiError extends Error {
  readonly code: string;
  readonly status: number;
  readonly field: string | undefined;

  constructor(status: number, body: ErrorBody) {
    super(body.message);
    this.name = 'ApiError';
    this.status = status;
    this.code = body.code;
    this.field = body.field;
  }

  /** Whether re-authenticating would plausibly fix this. */
  get isAuthFailure(): boolean {
    return this.code === 'not_authenticated' || this.status === 401;
  }
}

/**
 * The request never reached the server.
 *
 * Distinguished from {@link ApiError} because the recovery is different: waiting
 * and retrying, rather than correcting the input. In development this is the
 * normal state for the second or two while cargo-watch rebuilds.
 */
export class NetworkError extends Error {
  constructor(cause: unknown) {
    super('Could not reach the Elrond server.');
    this.name = 'NetworkError';
    this.cause = cause;
  }
}

/**
 * The CSRF token for this browsing session.
 *
 * Held in memory rather than read back out of the cookie on each call: the token
 * arrives in the body of every response that sets it, and keeping one source of
 * truth avoids a stale read after rotation.
 */
let csrfToken: string | null = null;

/** Records the CSRF token returned by bootstrap or a sign-in. */
export function rememberCsrfToken(token: string | null): void {
  csrfToken = token;
}

/** HTTP methods that change state and therefore need a CSRF token. */
const UNSAFE_METHODS = new Set(['POST', 'PUT', 'PATCH', 'DELETE']);

/** Performs a request and decodes the response. */
async function request<T>(
  path: string,
  options: {
    method?: string;
    body?: unknown;
    signal?: AbortSignal;
    /** Set false when the endpoint answers 204 with no body. */
    expectBody?: boolean;
  } = {},
): Promise<T> {
  const method = options.method ?? 'GET';
  const headers = new Headers();

  if (options.body !== undefined) {
    headers.set('Content-Type', 'application/json');
  }
  if (UNSAFE_METHODS.has(method) && csrfToken !== null) {
    headers.set(CSRF_HEADER, csrfToken);
  }

  let response: Response;
  try {
    response = await fetch(`${API_BASE}${path}`, {
      method,
      headers,
      // The session cookie is HttpOnly, so it can only travel this way.
      credentials: 'same-origin',
      body: options.body === undefined ? null : JSON.stringify(options.body),
      ...(options.signal ? { signal: options.signal } : {}),
    });
  } catch (cause) {
    // An aborted request is a caller decision, not a connectivity problem.
    if (cause instanceof DOMException && cause.name === 'AbortError') {
      throw cause;
    }
    throw new NetworkError(cause);
  }

  if (!response.ok) {
    throw new ApiError(response.status, await readErrorBody(response));
  }

  if (options.expectBody === false) {
    return undefined as T;
  }
  return (await response.json()) as T;
}

/**
 * Performs a request whose success carries no body.
 *
 * Separate from {@link request} so no caller has to name `void` as a type
 * argument, and so a 204 is never fed to `response.json()`.
 */
async function requestNoContent(
  path: string,
  options: { method: string; body?: unknown },
): Promise<void> {
  await request<unknown>(path, { ...options, expectBody: false });
}

/** Reads an error body, tolerating a response that is not the expected JSON. */
async function readErrorBody(response: Response): Promise<ErrorBody> {
  try {
    const parsed: unknown = await response.json();
    if (
      typeof parsed === 'object' &&
      parsed !== null &&
      'code' in parsed &&
      'message' in parsed &&
      typeof parsed.code === 'string' &&
      typeof parsed.message === 'string'
    ) {
      const field =
        'field' in parsed && typeof parsed.field === 'string' ? parsed.field : undefined;
      return { code: parsed.code, message: parsed.message, ...(field ? { field } : {}) };
    }
  } catch {
    // Fall through to the generic shape below.
  }

  // A proxy or a crash can produce an HTML error page. Reporting a usable code
  // beats surfacing a JSON parse failure the user cannot act on.
  return {
    code: 'unexpected_response',
    message: `The server returned an unexpected ${String(response.status)} response.`,
  };
}

/** Fields required to create the first administrator. */
export interface SetupInput {
  readonly username: string;
  readonly password: string;
}

/** Fields required to sign in. */
export interface SignInInput {
  readonly username: string;
  readonly password: string;
}

/** Server liveness and build version. */
export interface Health {
  readonly status: string;
  readonly version: string;
}

export const api = {
  /** Reads setup state, the current account, and a CSRF token. */
  async bootstrap(signal?: AbortSignal): Promise<Bootstrap> {
    const result = await request<Bootstrap>('/bootstrap', signal ? { signal } : {});
    rememberCsrfToken(result.csrf_token);
    return result;
  },

  /** Creates the first administrator and signs in as it. */
  async completeSetup(input: SetupInput): Promise<SessionCreated> {
    const result = await request<SessionCreated>('/setup', { method: 'POST', body: input });
    rememberCsrfToken(result.csrf_token);
    return result;
  },

  /** Signs in. */
  async signIn(input: SignInInput): Promise<SessionCreated> {
    const result = await request<SessionCreated>('/session', { method: 'POST', body: input });
    rememberCsrfToken(result.csrf_token);
    return result;
  },

  /** Signs out. Succeeds even with no session. */
  async signOut(): Promise<void> {
    await requestNoContent('/session', { method: 'DELETE' });
    rememberCsrfToken(null);
  },

  /** Reads the signed-in account. */
  me(): Promise<UserView> {
    return request<UserView>('/me');
  },

  /** Lists accounts. Administrators only. */
  listUsers(): Promise<UserView[]> {
    return request<UserView[]>('/users');
  },

  /** Checks that the server is up. Used to detect a restart in development. */
  health(signal?: AbortSignal): Promise<Health> {
    return request<Health>('/health', signal ? { signal } : {});
  },
};
