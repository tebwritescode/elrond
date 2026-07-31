import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useId, useRef, useState } from 'react';

import { Button, Callout, TextField } from '@/components';
import { api, type CategoryNode, type ImportResult, type UploadResult } from '@/lib/api';

import { partitionUploadError } from './queries';

/** Whether a chosen file is a ZIP archive, decided by its name. */
function isZip(file: File): boolean {
  return file.name.toLowerCase().endsWith('.zip');
}

export interface UploadFormProps {
  /** Category to file the upload under, or null for "Unfiled". */
  readonly categoryId: string | null;
  readonly categories: readonly CategoryNode[];
}

/**
 * Upload form.
 *
 * A plain file input rather than a drag-and-drop zone: a drop target that is not
 * also a real input is unreachable by keyboard, and this is the only ingestion
 * path at this milestone. Drag-and-drop is additive later.
 */
export function UploadForm({ categoryId, categories }: UploadFormProps) {
  const queryClient = useQueryClient();
  const fileInputId = useId();
  const fileInput = useRef<HTMLInputElement>(null);

  const [file, setFile] = useState<File | null>(null);
  const [title, setTitle] = useState('');
  const [tags, setTags] = useState('');
  const [result, setResult] = useState<UploadResult | null>(null);
  const [importResult, setImportResult] = useState<ImportResult | null>(null);

  const clearAfterSubmit = async () => {
    setFile(null);
    setTitle('');
    setTags('');
    // The native input keeps its selection after a successful submit, which
    // would let the same file be uploaded twice by accident.
    if (fileInput.current !== null) {
      fileInput.current.value = '';
    }
    await queryClient.invalidateQueries({ queryKey: ['documents'] });
    await queryClient.invalidateQueries({ queryKey: ['categories'] });
    await queryClient.invalidateQueries({ queryKey: ['tags'] });
  };

  const upload = useMutation({
    mutationFn: async (): Promise<
      { readonly imported: ImportResult } | { readonly uploaded: UploadResult }
    > => {
      if (file === null) {
        throw new Error('Choose a file to upload.');
      }
      // A ZIP goes to the importer: its folders become categories under the
      // selected one, and its files become documents.
      if (isZip(file)) {
        return { imported: await api.importZip(file, categoryId ?? undefined) };
      }
      return {
        uploaded: await api.uploadDocument({
          file,
          ...(categoryId === null ? {} : { categoryId }),
          ...(title.trim() === '' ? {} : { title: title.trim() }),
          tags: tags
            .split(',')
            .map((tag) => tag.trim())
            .filter((tag) => tag !== ''),
        }),
      };
    },
    onSuccess: async (outcome) => {
      if ('imported' in outcome) {
        setImportResult(outcome.imported);
        setResult(null);
      } else {
        setResult(outcome.uploaded);
        setImportResult(null);
      }
      await clearAfterSubmit();
    },
  });

  const { formError, fieldErrors } = partitionUploadError(upload.error);
  const destination =
    categoryId === null
      ? 'Unfiled'
      : (findCategory(categories, categoryId)?.name ?? 'the selected category');

  return (
    <form
      className="el-stack"
      style={{ gap: 'var(--el-space-4)' }}
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        upload.mutate();
      }}
    >
      {formError !== undefined && (
        <Callout tone="danger" title="Could not upload">
          {formError}
        </Callout>
      )}

      {result !== null && (
        <Callout
          tone={result.duplicate_of === null ? 'success' : 'caution'}
          title={
            result.duplicate_of === null
              ? `Uploaded ${result.document.title}`
              : 'Uploaded, but this content already exists'
          }
        >
          {result.duplicate_of === null
            ? 'Filed as a draft. Submit it for review when it is ready.'
            : 'Another document already has identical content. Both are kept, and the bytes are stored once.'}
        </Callout>
      )}

      {importResult !== null && (
        <Callout
          tone={importResult.imported.length > 0 ? 'success' : 'caution'}
          title={
            importResult.imported.length === 1
              ? 'Imported 1 document from the archive'
              : `Imported ${String(importResult.imported.length)} documents from the archive`
          }
        >
          Folders in the archive became categories.
          {importResult.skipped.length > 0 && (
            <>
              {' '}
              {importResult.skipped.length === 1
                ? '1 entry was skipped:'
                : `${String(importResult.skipped.length)} entries were skipped:`}
              <ul style={{ margin: 'var(--el-space-2) 0 0', paddingLeft: 'var(--el-space-4)' }}>
                {importResult.skipped.map((skip) => (
                  <li key={skip.path}>
                    <strong>{skip.path}</strong> — {skip.reason}
                  </li>
                ))}
              </ul>
            </>
          )}
        </Callout>
      )}

      <div className="el-field">
        <label className="el-field__label" htmlFor={fileInputId}>
          File
        </label>
        <p className="el-field__hint">
          PDF, images, office documents, or plain text. The original is stored byte-for-byte and
          never modified. A ZIP archive is imported whole: its folders become categories and its
          files become documents.
        </p>
        <input
          id={fileInputId}
          ref={fileInput}
          type="file"
          className="el-field__control"
          required
          aria-invalid={fieldErrors.file === undefined ? undefined : true}
          onChange={(event) => {
            setFile(event.target.files?.[0] ?? null);
          }}
        />
        <div role="alert" aria-live="polite">
          {fieldErrors.file !== undefined && (
            <p className="el-field__error">{fieldErrors.file}</p>
          )}
        </div>
      </div>

      {(file === null || !isZip(file)) && (
        <>
          <TextField
            label="Title"
            hint="Left empty, the filename is used."
            value={title}
            error={fieldErrors.title}
            onChange={(event) => {
              setTitle(event.target.value);
            }}
          />

          <TextField
            label="Tags"
            hint="Comma separated. Existing tags are reused regardless of capitalisation."
            value={tags}
            error={fieldErrors.tags}
            onChange={(event) => {
              setTags(event.target.value);
            }}
          />
        </>
      )}

      <p className="el-muted" style={{ fontSize: 'var(--el-text-xs)' }}>
        {file !== null && isZip(file) ? (
          <>
            Importing the archive into{' '}
            <strong>{categoryId === null ? 'the top level' : destination}</strong>. Its folder
            structure is created there.
          </>
        ) : (
          <>
            Filing into <strong>{destination}</strong>. Choose a different category in the tree
            to change this.
          </>
        )}
      </p>

      <Button
        type="submit"
        variant="primary"
        disabled={file === null}
        isLoading={upload.isPending}
        loadingLabel={file !== null && isZip(file) ? 'Importing' : 'Uploading'}
      >
        {file !== null && isZip(file) ? 'Import archive' : 'Upload document'}
      </Button>
    </form>
  );
}

/** Finds a category anywhere in the tree. */
function findCategory(
  categories: readonly CategoryNode[],
  id: string,
): CategoryNode | undefined {
  for (const category of categories) {
    if (category.id === id) {
      return category;
    }
    const found = findCategory(category.children, id);
    if (found !== undefined) {
      return found;
    }
  }
  return undefined;
}
