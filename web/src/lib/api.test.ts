import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ApiError, NetworkError, api, rememberCsrfToken } from './api';

/** Builds a `fetch` stub returning one JSON response. */
function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

/** Reads the headers off the single recorded fetch call. */
function recordedHeaders(mock: ReturnType<typeof vi.fn>): Headers {
  const init = mock.mock.calls[0]?.[1] as RequestInit | undefined;
  return new Headers(init?.headers);
}

describe('api client', () => {
  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
    rememberCsrfToken(null);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('calls the versioned API prefix', async () => {
    fetchMock.mockResolvedValue(jsonResponse({ status: 'ok', version: '0.1.0' }));
    await api.health();
    expect(fetchMock.mock.calls[0]?.[0]).toBe('/api/v1/health');
  });

  it('sends cookies, which is the only way the HttpOnly session can travel', async () => {
    fetchMock.mockResolvedValue(jsonResponse({ status: 'ok', version: '0.1.0' }));
    await api.health();
    const init = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect(init.credentials).toBe('same-origin');
  });

  it('remembers the CSRF token returned by bootstrap', async () => {
    fetchMock.mockResolvedValue(
      jsonResponse({
        requires_setup: true,
        user: null,
        csrf_token: 'token-from-bootstrap',
        version: '0.1.0',
      }),
    );
    const result = await api.bootstrap();
    expect(result.csrf_token).toBe('token-from-bootstrap');

    // The next unsafe request must echo it back.
    fetchMock.mockClear();
    fetchMock.mockResolvedValue(
      jsonResponse(
        { user: {}, csrf_token: 'rotated', expires_at: '2026-09-01T00:00:00Z' },
        201,
      ),
    );
    await api.signIn({ username: 'records.admin', password: 'a long passphrase' });
    expect(recordedHeaders(fetchMock).get('X-Elrond-CSRF')).toBe('token-from-bootstrap');
  });

  it('does not send a CSRF token on a safe request', async () => {
    rememberCsrfToken('some-token');
    fetchMock.mockResolvedValue(jsonResponse({ status: 'ok', version: '0.1.0' }));
    await api.health();
    expect(recordedHeaders(fetchMock).has('X-Elrond-CSRF')).toBe(false);
  });

  it('rotates the stored token when sign-in returns a new one', async () => {
    rememberCsrfToken('old-token');
    fetchMock.mockResolvedValue(
      jsonResponse(
        { user: {}, csrf_token: 'new-token', expires_at: '2026-09-01T00:00:00Z' },
        201,
      ),
    );
    await api.signIn({ username: 'records.admin', password: 'a long passphrase' });

    fetchMock.mockClear();
    fetchMock.mockResolvedValue(new Response(null, { status: 204 }));
    await api.signOut();
    expect(recordedHeaders(fetchMock).get('X-Elrond-CSRF')).toBe('new-token');
  });

  it('turns an error body into an ApiError carrying code and field', async () => {
    fetchMock.mockResolvedValue(
      jsonResponse(
        {
          code: 'field_too_short',
          message: 'password must be at least 12 characters',
          field: 'password',
        },
        422,
      ),
    );

    const error = await api
      .completeSetup({ username: 'records.admin', password: 'short' })
      .catch((caught: unknown) => caught);

    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).code).toBe('field_too_short');
    expect((error as ApiError).field).toBe('password');
    expect((error as ApiError).status).toBe(422);
  });

  it('recognises an authentication failure', async () => {
    fetchMock.mockResolvedValue(
      jsonResponse({ code: 'not_authenticated', message: 'authentication required' }, 401),
    );
    const error = (await api.me().catch((caught: unknown) => caught)) as ApiError;
    expect(error.isAuthFailure).toBe(true);
  });

  it('survives a non-JSON error page instead of throwing a parse error', async () => {
    // A crashed process or a proxy can answer with HTML. The client must still
    // produce something the interface can display.
    fetchMock.mockResolvedValue(
      new Response('<html><body>502 Bad Gateway</body></html>', {
        status: 502,
        headers: { 'Content-Type': 'text/html' },
      }),
    );

    const error = (await api.me().catch((caught: unknown) => caught)) as ApiError;
    expect(error).toBeInstanceOf(ApiError);
    expect(error.code).toBe('unexpected_response');
    expect(error.message).toContain('502');
  });

  it('reports a connectivity failure as a NetworkError, not an ApiError', async () => {
    fetchMock.mockRejectedValue(new TypeError('Failed to fetch'));
    const error = await api.health().catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(NetworkError);
    expect(error).not.toBeInstanceOf(ApiError);
  });

  it('lets an abort propagate rather than disguising it as a connectivity failure', async () => {
    fetchMock.mockRejectedValue(new DOMException('The operation was aborted.', 'AbortError'));
    const error = await api.health().catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(DOMException);
    expect(error).not.toBeInstanceOf(NetworkError);
  });

  it('does not try to parse a body on a 204', async () => {
    rememberCsrfToken('token');
    fetchMock.mockResolvedValue(new Response(null, { status: 204 }));
    await expect(api.signOut()).resolves.toBeUndefined();
  });
});
