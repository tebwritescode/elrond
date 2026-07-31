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

/** Where a document sits in its editorial workflow. */
export type Lifecycle = 'draft' | 'in_review' | 'published' | 'superseded' | 'archived';

/** A node in the category tree, with its children nested. */
export interface CategoryNode {
  readonly id: string;
  readonly name: string;
  /** Documents filed directly here. */
  readonly document_count: number;
  /** Documents here and everywhere beneath. */
  readonly total_document_count: number;
  readonly children: readonly CategoryNode[];
}

/** A tag with how many documents carry it. */
export interface TagCount {
  readonly id: string;
  readonly label: string;
  readonly document_count: number;
}

/** One immutable version of a document. */
export interface VersionView {
  readonly id: string;
  readonly number: number;
  readonly filename: string;
  readonly media_type: string;
  readonly byte_size: number;
  readonly checksum: string;
  /** Whether a PDF is available to view. */
  readonly has_pdf: boolean;
  /** Whether a PDF copy still has to be generated. */
  readonly awaiting_conversion: boolean;
  readonly note: string | null;
  readonly created_at: string;
}

/** A document as the API renders it. Never carries a storage key. */
export interface DocumentView {
  readonly id: string;
  readonly title: string;
  readonly category_id: string;
  readonly category_name: string;
  readonly lifecycle: Lifecycle;
  readonly version_count: number;
  readonly current_version: VersionView;
  readonly tags: readonly { readonly id: string; readonly label: string }[];
  readonly source_path: string | null;
  readonly review_due_at: string | null;
  readonly created_at: string;
  readonly updated_at: string;
}

/** One page of a listing. */
export interface DocumentPage {
  readonly documents: readonly DocumentView[];
  readonly total: number;
  readonly limit: number;
  readonly offset: number;
}

/** A document with its full version history. */
export type DocumentDetail = DocumentView & { readonly versions: readonly VersionView[] };

/** The result of an upload. */
export interface UploadResult {
  readonly document: DocumentView;
  /** Whether the bytes were already stored and were reused. */
  readonly deduplicated: boolean;
  /** An existing document with identical content, if any. */
  readonly duplicate_of: string | null;
}

/** One archive entry that was not imported, and why. */
export interface ImportSkip {
  readonly path: string;
  readonly reason: string;
}

/** The result of a ZIP import. */
export interface ImportResult {
  readonly imported: readonly DocumentView[];
  readonly skipped: readonly ImportSkip[];
}

/** Filters for a document listing. */
export interface DocumentQuery {
  readonly q?: string;
  readonly categoryId?: string;
  readonly includeDescendants?: boolean;
  readonly tagIds?: readonly string[];
  readonly sort?: 'title' | 'created' | 'updated' | 'size' | 'relevance';
  readonly order?: 'asc' | 'desc';
  readonly limit?: number;
  readonly offset?: number;
}

/** Renders a listing filter as a query string. */
function toSearchParams(query: DocumentQuery): string {
  const params = new URLSearchParams();
  if (query.q) params.set('q', query.q);
  if (query.categoryId) params.set('category_id', query.categoryId);
  if (query.includeDescendants === false) params.set('include_descendants', 'false');
  if (query.tagIds && query.tagIds.length > 0) params.set('tags', query.tagIds.join(','));
  if (query.sort) params.set('sort', query.sort);
  if (query.order) params.set('order', query.order);
  if (query.limit !== undefined) params.set('limit', String(query.limit));
  if (query.offset !== undefined) params.set('offset', String(query.offset));
  const rendered = params.toString();
  return rendered === '' ? '' : `?${rendered}`;
}

/** Fields an upload can carry alongside the file. */
export interface UploadFields {
  readonly file: File;
  readonly categoryId?: string | undefined;
  readonly title?: string | undefined;
  readonly tags?: readonly string[] | undefined;
}

/**
 * Performs a multipart upload.
 *
 * `Content-Type` is deliberately not set: the browser has to add it itself so the
 * generated multipart boundary matches the body it produced.
 */
async function postMultipart<T>(path: string, fields: UploadFields): Promise<T> {
  const form = new FormData();
  form.set('file', fields.file, fields.file.name);
  if (fields.categoryId) form.set('category_id', fields.categoryId);
  if (fields.title) form.set('title', fields.title);
  if (fields.tags && fields.tags.length > 0) form.set('tags', fields.tags.join(','));

  const headers = new Headers();
  if (csrfToken !== null) {
    headers.set(CSRF_HEADER, csrfToken);
  }

  let response: Response;
  try {
    response = await fetch(`${API_BASE}${path}`, {
      method: 'POST',
      headers,
      credentials: 'same-origin',
      body: form,
    });
  } catch (cause) {
    throw new NetworkError(cause);
  }

  if (!response.ok) {
    throw new ApiError(response.status, await readErrorBody(response));
  }
  return (await response.json()) as T;
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

  /** Reads the whole category tree with document counts. */
  categories(): Promise<CategoryNode[]> {
    return request<CategoryNode[]>('/categories');
  },

  /** Creates a category. Refuses a duplicate sibling name. */
  createCategory(name: string, parentId?: string): Promise<{ readonly id: string }> {
    return request('/categories', {
      method: 'POST',
      body: parentId === undefined ? { name } : { name, parent_id: parentId },
    });
  },

  /** Renames a category. */
  renameCategory(
    id: string,
    name: string,
  ): Promise<{ readonly id: string; readonly name: string }> {
    return request(`/categories/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      body: { name },
    });
  },

  /** Deletes an empty category. */
  deleteCategory(id: string): Promise<void> {
    return requestNoContent(`/categories/${encodeURIComponent(id)}`, { method: 'DELETE' });
  },

  /** Reads every tag with its document count. */
  tags(): Promise<TagCount[]> {
    return request<TagCount[]>('/tags');
  },

  /** Lists documents, optionally narrowed by a search query. */
  documents(query: DocumentQuery = {}): Promise<DocumentPage> {
    return request<DocumentPage>(`/documents${toSearchParams(query)}`);
  },

  /** Reads one document with its version history. */
  document(id: string): Promise<DocumentDetail> {
    return request<DocumentDetail>(`/documents/${encodeURIComponent(id)}`);
  },

  /** Uploads a new document. */
  uploadDocument(fields: UploadFields): Promise<UploadResult> {
    return postMultipart<UploadResult>('/documents', fields);
  },

  /**
   * Imports a ZIP archive: folders become categories, files become documents.
   * Unsupported entries are skipped and reported rather than failing the whole
   * archive.
   */
  importZip(file: File, categoryId?: string): Promise<ImportResult> {
    return postMultipart<ImportResult>('/documents/import', { file, categoryId });
  },

  /** Appends a version to an existing document. */
  addVersion(id: string, fields: UploadFields): Promise<VersionView> {
    return postMultipart<VersionView>(`/documents/${encodeURIComponent(id)}/versions`, fields);
  },

  /** Moves a document through its lifecycle. */
  transition(id: string, lifecycle: Lifecycle): Promise<DocumentView> {
    return request<DocumentView>(`/documents/${encodeURIComponent(id)}/lifecycle`, {
      method: 'POST',
      body: { lifecycle },
    });
  },

  /** Updates title, category, review date, and tags. */
  updateDocument(
    id: string,
    body: {
      readonly title: string;
      readonly category_id: string;
      readonly review_due_at?: string | null;
      readonly tags: readonly string[];
    },
  ): Promise<DocumentView> {
    return request<DocumentView>(`/documents/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      body,
    });
  },
};

/** Options for generating a binder. */
export interface BuildBinderOptions {
  readonly title: string;
  readonly subtitle?: string;
  readonly organization?: string;
  /** Empty means the whole library. A category always brings its descendants. */
  readonly category_ids: readonly string[];
  readonly page_size: 'a4' | 'letter';
  readonly include_cover: boolean;
  readonly include_toc: boolean;
  readonly include_separators: boolean;
  readonly document_separators: boolean;
  readonly page_numbers: boolean;
  readonly duplex_blank_pages: boolean;
}

/** A generated binder, ready to hand to the browser's download machinery. */
export interface BuiltBinder {
  readonly blob: Blob;
  readonly filename: string;
  readonly pageCount: number;
  readonly documentCount: number;
  /** How many documents were left out for want of a PDF. */
  readonly skipped: number;
}

/**
 * Generates a binder and returns the PDF.
 *
 * Fetched rather than submitted as a plain form, because the request needs the
 * CSRF header and a form post cannot set one. The counts come back as headers so
 * the body can stay a raw PDF.
 */
export async function buildBinder(options: BuildBinderOptions): Promise<BuiltBinder> {
  const headers = new Headers({ 'Content-Type': 'application/json' });
  if (csrfToken !== null) {
    headers.set(CSRF_HEADER, csrfToken);
  }

  let response: Response;
  try {
    response = await fetch(`${API_BASE}/binders/build`, {
      method: 'POST',
      headers,
      credentials: 'same-origin',
      body: JSON.stringify(options),
    });
  } catch (cause) {
    throw new NetworkError(cause);
  }

  if (!response.ok) {
    throw new ApiError(response.status, await readErrorBody(response));
  }

  const disposition = response.headers.get('content-disposition') ?? '';
  const match = /filename="([^"]+)"/.exec(disposition);

  return {
    blob: await response.blob(),
    filename: match?.[1] ?? 'binder.pdf',
    pageCount: Number(response.headers.get('x-elrond-page-count') ?? '0'),
    documentCount: Number(response.headers.get('x-elrond-document-count') ?? '0'),
    skipped: Number(response.headers.get('x-elrond-skipped-count') ?? '0'),
  };
}

/** URL that downloads a version's immutable original. */
export function originalUrl(versionId: string): string {
  return `${API_BASE}/versions/${encodeURIComponent(versionId)}/original`;
}

/** URL that serves a version's PDF inline, for the viewer. */
export function pdfUrl(versionId: string): string {
  return `${API_BASE}/versions/${encodeURIComponent(versionId)}/pdf`;
}
