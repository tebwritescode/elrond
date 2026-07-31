import { useMutation } from '@tanstack/react-query';
import { useState } from 'react';

import { Button, Callout, Panel, TextField } from '@/components';
import { PageHeader } from '@/components/PageHeader';
import { useCategories } from '@/features/documents/queries';
import { buildBinder, type BuildBinderOptions, type CategoryNode } from '@/lib/api';

/** Flattens the tree for a checkbox list, keeping depth for indentation. */
function flatten(
  categories: readonly CategoryNode[],
  depth = 0,
): { node: CategoryNode; depth: number }[] {
  return categories.flatMap((node) => [{ node, depth }, ...flatten(node.children, depth + 1)]);
}

/**
 * Binder generation.
 *
 * A binder is produced on demand from the category tree rather than assembled and
 * saved first. That is what makes it useful immediately: upload, categorise, and
 * print the result, with no separate outline to maintain.
 */
export function BindersPage() {
  const categories = useCategories();
  const rows = flatten(categories.data ?? []);

  const [title, setTitle] = useState('Document Binder');
  const [subtitle, setSubtitle] = useState('');
  const [organization, setOrganization] = useState('');
  const [selected, setSelected] = useState<readonly string[]>([]);
  const [includeCover, setIncludeCover] = useState(true);
  const [includeToc, setIncludeToc] = useState(true);
  const [includeSeparators, setIncludeSeparators] = useState(true);
  const [pageNumbers, setPageNumbers] = useState(true);
  const [duplex, setDuplex] = useState(false);
  const [pageSize, setPageSize] = useState<'a4' | 'letter'>('a4');
  const [result, setResult] = useState<{
    pages: number;
    documents: number;
    skipped: number;
  } | null>(null);

  const build = useMutation({
    mutationFn: async () => {
      const options: BuildBinderOptions = {
        title: title.trim(),
        ...(subtitle.trim() === '' ? {} : { subtitle: subtitle.trim() }),
        ...(organization.trim() === '' ? {} : { organization: organization.trim() }),
        category_ids: selected,
        page_size: pageSize,
        include_cover: includeCover,
        include_toc: includeToc,
        include_separators: includeSeparators,
        page_numbers: pageNumbers,
        duplex_blank_pages: duplex,
      };
      return buildBinder(options);
    },
    onSuccess: (built) => {
      setResult({
        pages: built.pageCount,
        documents: built.documentCount,
        skipped: built.skipped,
      });

      // Handing the blob to a temporary anchor is what lets the browser do its own
      // download handling; fetching was necessary because the request needs the
      // CSRF header, which a plain form post cannot set.
      const url = URL.createObjectURL(built.blob);
      const anchor = document.createElement('a');
      anchor.href = url;
      anchor.download = built.filename;
      document.body.append(anchor);
      anchor.click();
      anchor.remove();
      // Revoked on the next tick so the download has certainly started.
      setTimeout(() => {
        URL.revokeObjectURL(url);
      }, 1000);
    },
  });

  const toggle = (id: string) => {
    setSelected((current) =>
      current.includes(id) ? current.filter((value) => value !== id) : [...current, id],
    );
  };

  return (
    <div className="el-stack">
      <PageHeader
        eyebrow="Publishing"
        title="Binders"
        lede="Generate one printable PDF containing every document in the categories you choose, with a cover, a full-page separator introducing each category, a table of contents, and page numbers."
      />

      {build.isError && (
        <Callout tone="danger" title="Could not build the binder">
          {build.error.message}
        </Callout>
      )}

      {result !== null && (
        <Callout
          tone={result.skipped > 0 ? 'caution' : 'success'}
          title={`Built ${String(result.pages)} pages from ${String(result.documents)} documents`}
        >
          {result.skipped > 0
            ? `Your download has started. ${String(result.skipped)} document${result.skipped === 1 ? '' : 's'} were left out because no PDF has been generated for them yet — only PDFs can be bound at this milestone.`
            : 'Your download has started.'}
        </Callout>
      )}

      <div className="el-binder-layout">
        <Panel title="Contents">
          <fieldset style={{ border: 0, margin: 0, padding: 0 }}>
            <legend className="el-field__hint" style={{ marginBottom: 'var(--el-space-3)' }}>
              Leave everything unticked to bind the whole library. Ticking a category always
              includes the categories nested inside it.
            </legend>

            {categories.isPending && <p className="el-muted">Loading categories…</p>}

            {!categories.isPending && rows.length === 0 && (
              <p className="el-muted">
                No categories yet. Upload a document first and it will be filed under “Unfiled”.
              </p>
            )}

            <div className="el-stack" style={{ gap: 'var(--el-space-1)' }}>
              {rows.map(({ node, depth }) => (
                <label
                  key={node.id}
                  className="el-checkbox"
                  style={{ marginLeft: `${String(depth * 1.25)}rem` }}
                >
                  <input
                    type="checkbox"
                    checked={selected.includes(node.id)}
                    onChange={() => {
                      toggle(node.id);
                    }}
                  />
                  <span>{node.name}</span>
                  <span className="el-tree__count">{node.total_document_count}</span>
                </label>
              ))}
            </div>
          </fieldset>
        </Panel>

        <Panel title="Cover and layout">
          <form
            className="el-stack"
            style={{ gap: 'var(--el-space-4)' }}
            noValidate
            onSubmit={(event) => {
              event.preventDefault();
              build.mutate();
            }}
          >
            <TextField
              label="Title"
              required
              value={title}
              onChange={(event) => {
                setTitle(event.target.value);
              }}
            />
            <TextField
              label="Subtitle"
              value={subtitle}
              onChange={(event) => {
                setSubtitle(event.target.value);
              }}
            />
            <TextField
              label="Organisation"
              value={organization}
              onChange={(event) => {
                setOrganization(event.target.value);
              }}
            />

            <div className="el-field">
              <label className="el-field__label" htmlFor="binder-page-size">
                Paper size
              </label>
              <select
                id="binder-page-size"
                className="el-field__control"
                value={pageSize}
                onChange={(event) => {
                  setPageSize(event.target.value === 'letter' ? 'letter' : 'a4');
                }}
              >
                <option value="a4">A4</option>
                <option value="letter">Letter</option>
              </select>
            </div>

            <fieldset style={{ border: 0, margin: 0, padding: 0 }}>
              <legend className="el-field__label" style={{ marginBottom: 'var(--el-space-2)' }}>
                Include
              </legend>
              <div className="el-stack" style={{ gap: 'var(--el-space-1)' }}>
                <Toggle label="Front cover" checked={includeCover} onChange={setIncludeCover} />
                <Toggle
                  label="Table of contents"
                  checked={includeToc}
                  onChange={setIncludeToc}
                />
                <Toggle
                  label="Full-page category separators"
                  checked={includeSeparators}
                  onChange={setIncludeSeparators}
                />
                <Toggle label="Page numbers" checked={pageNumbers} onChange={setPageNumbers} />
                <Toggle
                  label="Blank pages for double-sided printing"
                  hint="Pads so every separator falls on a right-hand page."
                  checked={duplex}
                  onChange={setDuplex}
                />
              </div>
            </fieldset>

            <Button
              type="submit"
              variant="primary"
              size="lg"
              disabled={title.trim() === ''}
              isLoading={build.isPending}
              loadingLabel="Building the binder"
            >
              Build and download
            </Button>
          </form>
        </Panel>
      </div>
    </div>
  );
}

/** A labelled checkbox with an optional hint. */
function Toggle({
  label,
  hint,
  checked,
  onChange,
}: {
  readonly label: string;
  readonly hint?: string;
  readonly checked: boolean;
  readonly onChange: (next: boolean) => void;
}) {
  return (
    <label className="el-checkbox">
      <input
        type="checkbox"
        checked={checked}
        onChange={(event) => {
          onChange(event.target.checked);
        }}
      />
      <span>
        {label}
        {hint !== undefined && <span className="el-field__hint"> {hint}</span>}
      </span>
    </label>
  );
}
