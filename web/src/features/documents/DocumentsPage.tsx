import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';

import { Button, Callout, EmptyState, Panel, Pill, Skeleton, TextField } from '@/components';
import { PageHeader } from '@/components/PageHeader';
import { useBootstrap } from '@/features/auth/session';
import {
  api,
  originalUrl,
  pdfUrl,
  type DocumentQuery,
  type DocumentView,
  type Lifecycle,
} from '@/lib/api';

import { CategoryForm } from './CategoryForm';
import { CategoryManager } from './CategoryManager';
import { CategoryTree } from './CategoryTree';
import { DocumentInfoPanel } from './DocumentInfoPanel';
import {
  LIFECYCLE_LABELS,
  LIFECYCLE_TONES,
  formatBytes,
  formatDate,
  useCategories,
  useDocuments,
  useTags,
} from './queries';
import { UploadForm } from './UploadForm';

/** Rows per page. */
const PAGE_SIZE = 25;

/** The document library: category tree, filters, and a sortable table. */
export function DocumentsPage() {
  const bootstrap = useBootstrap();
  const role = bootstrap.data?.user?.role ?? 'viewer';
  const canWrite = role === 'editor' || role === 'admin';

  const [categoryId, setCategoryId] = useState<string | null>(null);
  const [searchInput, setSearchInput] = useState('');
  const [search, setSearch] = useState('');
  const [selectedTags, setSelectedTags] = useState<readonly string[]>([]);
  const [sort, setSort] = useState<NonNullable<DocumentQuery['sort']>>('updated');
  const [order, setOrder] = useState<NonNullable<DocumentQuery['order']>>('desc');
  const [page, setPage] = useState(0);
  const [showUpload, setShowUpload] = useState(false);

  // Debounced so typing does not issue a request per keystroke, and so the
  // relevance ordering does not thrash while a word is half-typed.
  useEffect(() => {
    const timer = setTimeout(() => {
      setSearch(searchInput.trim());
      setPage(0);
    }, 250);
    return () => {
      clearTimeout(timer);
    };
  }, [searchInput]);

  const query: DocumentQuery = {
    ...(search === '' ? {} : { q: search }),
    ...(categoryId === null ? {} : { categoryId }),
    ...(selectedTags.length === 0 ? {} : { tagIds: selectedTags }),
    // With a query, relevance is the only ordering that makes sense.
    sort: search === '' ? sort : 'relevance',
    order,
    limit: PAGE_SIZE,
    offset: page * PAGE_SIZE,
  };

  const categories = useCategories();
  const tags = useTags();
  const documents = useDocuments(query);

  const total = documents.data?.total ?? 0;
  const pageCount = Math.max(1, Math.ceil(total / PAGE_SIZE));

  return (
    <div className="el-library">
      <aside className="el-library__aside">
        {categories.isPending ? (
          <div className="el-stack" style={{ gap: 'var(--el-space-2)' }}>
            <Skeleton height="1.5rem" />
            <Skeleton height="1.5rem" width="80%" />
            <Skeleton height="1.5rem" width="60%" />
          </div>
        ) : (
          <CategoryTree
            categories={categories.data ?? []}
            selectedId={categoryId}
            total={total}
            onSelect={(id) => {
              setCategoryId(id);
              setPage(0);
            }}
          />
        )}

        {canWrite && (
          <div
            className="el-stack"
            style={{ marginTop: 'var(--el-space-4)', gap: 'var(--el-space-3)' }}
          >
            <CategoryForm parentId={categoryId} categories={categories.data ?? []} />
            {categoryId !== null && (
              <CategoryManager
                key={categoryId}
                categoryId={categoryId}
                categories={categories.data ?? []}
                onDeleted={() => {
                  setCategoryId(null);
                }}
              />
            )}
          </div>
        )}

        {(tags.data ?? []).length > 0 && (
          <section style={{ marginTop: 'var(--el-space-5)' }}>
            <h2 className="el-eyebrow" style={{ marginBottom: 'var(--el-space-2)' }}>
              Tags
            </h2>
            <div className="el-tag-filter" role="group" aria-label="Filter by tag">
              {(tags.data ?? []).map((tag) => {
                const active = selectedTags.includes(tag.id);
                return (
                  <button
                    key={tag.id}
                    type="button"
                    className="el-tag-filter__tag"
                    aria-pressed={active}
                    onClick={() => {
                      setSelectedTags((current) =>
                        active ? current.filter((id) => id !== tag.id) : [...current, tag.id],
                      );
                      setPage(0);
                    }}
                  >
                    {tag.label}
                    <span className="el-tree__count">{tag.document_count}</span>
                  </button>
                );
              })}
            </div>
            {selectedTags.length > 1 && (
              <p
                className="el-muted"
                style={{ fontSize: 'var(--el-text-xs)', marginTop: 'var(--el-space-2)' }}
              >
                Documents must carry every selected tag.
              </p>
            )}
          </section>
        )}
      </aside>

      <div className="el-library__main el-stack">
        <PageHeader
          eyebrow="Library"
          title="Documents"
          lede="Originals are stored byte-for-byte and never modified. Replacing content appends a new version, so anything a binder release already pins stays intact."
          action={
            canWrite ? (
              <Button
                variant="primary"
                onClick={() => {
                  setShowUpload((current) => !current);
                }}
                aria-expanded={showUpload}
              >
                {showUpload ? 'Close upload' : 'Upload document'}
              </Button>
            ) : undefined
          }
        />

        {showUpload && canWrite && (
          <Panel title="Upload">
            <UploadForm categoryId={categoryId} categories={categories.data ?? []} />
          </Panel>
        )}

        <div className="el-toolbar">
          <div style={{ flex: '1 1 18rem', minWidth: 0 }}>
            <TextField
              label="Search"
              type="search"
              placeholder="Title, filename, or tag"
              hint="Matches every word you type. Punctuation is ignored."
              value={searchInput}
              onChange={(event) => {
                setSearchInput(event.target.value);
              }}
            />
          </div>

          <div className="el-field" style={{ flex: '0 0 auto' }}>
            <label className="el-field__label" htmlFor="sort-column">
              Sort by
            </label>
            <select
              id="sort-column"
              className="el-field__control"
              value={search === '' ? sort : 'relevance'}
              disabled={search !== ''}
              onChange={(event) => {
                setSort(event.target.value as NonNullable<DocumentQuery['sort']>);
                setPage(0);
              }}
            >
              <option value="updated">Last changed</option>
              <option value="created">Date added</option>
              <option value="title">Title</option>
              <option value="size">Size</option>
              {search !== '' && <option value="relevance">Relevance</option>}
            </select>
          </div>

          <div className="el-field" style={{ flex: '0 0 auto' }}>
            <label className="el-field__label" htmlFor="sort-order">
              Order
            </label>
            <select
              id="sort-order"
              className="el-field__control"
              value={order}
              onChange={(event) => {
                setOrder(event.target.value as NonNullable<DocumentQuery['order']>);
                setPage(0);
              }}
            >
              <option value="desc">Descending</option>
              <option value="asc">Ascending</option>
            </select>
          </div>
        </div>

        {documents.isError && (
          <Callout tone="danger" title="Could not load documents">
            {documents.error.message}
          </Callout>
        )}

        <Panel
          title={total === 1 ? '1 document' : `${String(total)} documents`}
          flush
          action={
            pageCount > 1 ? (
              <div className="el-row" style={{ gap: 'var(--el-space-2)' }}>
                <Button
                  size="sm"
                  disabled={page === 0}
                  onClick={() => {
                    setPage((current) => Math.max(0, current - 1));
                  }}
                >
                  Previous
                </Button>
                <span className="el-muted" style={{ fontSize: 'var(--el-text-sm)' }}>
                  Page {page + 1} of {pageCount}
                </span>
                <Button
                  size="sm"
                  disabled={page + 1 >= pageCount}
                  onClick={() => {
                    setPage((current) => current + 1);
                  }}
                >
                  Next
                </Button>
              </div>
            ) : undefined
          }
        >
          {documents.isPending ? (
            <div
              className="el-stack"
              style={{ gap: 'var(--el-space-3)', padding: 'var(--el-space-5)' }}
            >
              <Skeleton height="1.25rem" width="45%" />
              <Skeleton height="1.25rem" width="70%" />
              <Skeleton height="1.25rem" width="55%" />
            </div>
          ) : (
            <DocumentTable
              documents={documents.data?.documents ?? []}
              canWrite={canWrite}
              hasFilters={search !== '' || categoryId !== null || selectedTags.length > 0}
              onClearFilters={() => {
                setSearchInput('');
                setSearch('');
                setCategoryId(null);
                setSelectedTags([]);
                setPage(0);
              }}
            />
          )}
        </Panel>
      </div>
    </div>
  );
}

/** The document table, or an explanation of why it is empty. */
function DocumentTable({
  documents,
  canWrite,
  hasFilters,
  onClearFilters,
}: {
  readonly documents: readonly DocumentView[];
  readonly canWrite: boolean;
  readonly hasFilters: boolean;
  readonly onClearFilters: () => void;
}) {
  const queryClient = useQueryClient();
  const [infoId, setInfoId] = useState<string | null>(null);
  const transition = useMutation({
    mutationFn: ({ id, lifecycle }: { id: string; lifecycle: Lifecycle }) =>
      api.transition(id, lifecycle),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['documents'] });
    },
  });

  if (documents.length === 0) {
    return hasFilters ? (
      <EmptyState
        title="Nothing matches those filters"
        action={
          <Button variant="secondary" onClick={onClearFilters}>
            Clear filters
          </Button>
        }
      >
        Try a shorter search, a different category, or fewer tags. Search requires every word
        you type to appear somewhere in the document.
      </EmptyState>
    ) : (
      <EmptyState title="The library is empty">
        {canWrite
          ? 'Upload a document to get started. Originals are preserved exactly as supplied, and a PDF copy is generated for anything that is not already a PDF.'
          : 'No published documents yet. Documents become visible here once a reviewer has approved them.'}
      </EmptyState>
    );
  }

  return (
    <>
      {transition.isError && (
        <div style={{ padding: 'var(--el-space-4) var(--el-space-5) 0' }}>
          <Callout tone="danger" title="Could not change the state">
            {transition.error.message}
          </Callout>
        </div>
      )}
      {infoId !== null && (
        <div style={{ padding: 'var(--el-space-4) var(--el-space-5) 0' }}>
          <DocumentInfoPanel
            documentId={infoId}
            onClose={() => {
              setInfoId(null);
            }}
          />
        </div>
      )}
      <div className="el-table-wrap">
        <table className="el-table">
          <caption className="el-visually-hidden">
            Documents, with category, state, size, and version count
          </caption>
          <thead>
            <tr>
              <th scope="col">Title</th>
              <th scope="col">Category</th>
              <th scope="col">State</th>
              <th scope="col">Tags</th>
              <th scope="col">Size</th>
              <th scope="col">Versions</th>
              <th scope="col">Changed</th>
              <th scope="col">
                <span className="el-visually-hidden">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {documents.map((document) => (
              <tr key={document.id}>
                <th scope="row" className="el-table__title">
                  {document.current_version.has_pdf ? (
                    // Clicking a document opens the document. Reading about it
                    // is the ⓘ button's job.
                    <a
                      className="el-table__open"
                      href={pdfUrl(document.current_version.id)}
                      target="_blank"
                      rel="noreferrer"
                    >
                      {document.title}
                    </a>
                  ) : (
                    document.title
                  )}
                  <span className="el-table__filename">
                    {document.current_version.filename}
                  </span>
                </th>
                <td>{document.category_name}</td>
                <td>
                  <Pill tone={LIFECYCLE_TONES[document.lifecycle] ?? 'neutral'}>
                    {LIFECYCLE_LABELS[document.lifecycle] ?? document.lifecycle}
                  </Pill>
                </td>
                <td>
                  {document.tags.length === 0 ? (
                    <span className="el-muted">—</span>
                  ) : (
                    <span
                      className="el-row"
                      style={{ gap: 'var(--el-space-1)', flexWrap: 'wrap' }}
                    >
                      {document.tags.map((tag) => (
                        <Pill key={tag.id}>{tag.label}</Pill>
                      ))}
                    </span>
                  )}
                </td>
                <td>{formatBytes(document.current_version.byte_size)}</td>
                <td>{document.version_count}</td>
                <td>{formatDate(document.updated_at)}</td>
                <td>
                  <span className="el-row" style={{ gap: 'var(--el-space-2)' }}>
                    <Button
                      size="sm"
                      variant="ghost"
                      aria-label={`Details for ${document.title}`}
                      onClick={() => {
                        setInfoId((current) => (current === document.id ? null : document.id));
                      }}
                    >
                      Info
                    </Button>
                    {/*
                      A plain link rather than a fetch: the browser's own download
                      handling is better than anything reimplemented, and the
                      response is an attachment so the page does not navigate.
                    */}
                    <a
                      className="el-button el-button--secondary el-button--sm"
                      href={originalUrl(document.current_version.id)}
                    >
                      Download
                    </a>
                    {canWrite && document.lifecycle === 'draft' && (
                      <Button
                        size="sm"
                        isLoading={
                          transition.isPending && transition.variables.id === document.id
                        }
                        loadingLabel="Submitting"
                        onClick={() => {
                          transition.mutate({ id: document.id, lifecycle: 'in_review' });
                        }}
                      >
                        Submit for review
                      </Button>
                    )}
                    {canWrite && document.lifecycle === 'in_review' && (
                      <Button
                        size="sm"
                        variant="primary"
                        isLoading={
                          transition.isPending && transition.variables.id === document.id
                        }
                        loadingLabel="Publishing"
                        onClick={() => {
                          transition.mutate({ id: document.id, lifecycle: 'published' });
                        }}
                      >
                        Publish
                      </Button>
                    )}
                  </span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </>
  );
}
