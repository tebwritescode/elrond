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

/** What one submit produced: plain uploads, or an archive import. */
type SubmitOutcome =
  { readonly uploaded: readonly UploadResult[] } | { readonly imported: ImportResult };

/**
 * Upload form.
 *
 * A plain file input rather than a drag-and-drop zone: a drop target that is not
 * also a real input is unreachable by keyboard, and this is the only ingestion
 * path at this milestone. Drag-and-drop is additive later.
 *
 * The input accepts several files at once; each becomes its own document, so a
 * folder's worth of PDFs does not need a round trip per file. A single `.zip`
 * goes to the importer instead, which also recreates the archive's folders.
 */
export function UploadForm({ categoryId, categories }: UploadFormProps) {
  const queryClient = useQueryClient();
  const fileInputId = useId();
  const fileInput = useRef<HTMLInputElement>(null);

  const [files, setFiles] = useState<readonly File[]>([]);
  const [title, setTitle] = useState('');
  const [tags, setTags] = useState('');
  const [outcome, setOutcome] = useState<SubmitOutcome | null>(null);

  const single = files.length === 1 ? files[0] : undefined;
  const zipChosen = single !== undefined && isZip(single);

  const upload = useMutation({
    mutationFn: async (): Promise<SubmitOutcome> => {
      if (files.length === 0) {
        throw new Error('Choose a file to upload.');
      }
      // A lone ZIP goes to the importer: its folders become categories under
      // the selected one, and its files become documents.
      if (single !== undefined && isZip(single)) {
        return { imported: await api.importZip(single, categoryId ?? undefined) };
      }

      const parsedTags = tags
        .split(',')
        .map((tag) => tag.trim())
        .filter((tag) => tag !== '');

      // Sequential on purpose: parallel uploads would race on tag creation,
      // and the practical difference for a handful of files is nothing.
      const uploaded: UploadResult[] = [];
      for (const file of files) {
        uploaded.push(
          await api.uploadDocument({
            file,
            ...(categoryId === null ? {} : { categoryId }),
            // A typed title only makes sense for a single file; with several,
            // each filename is its title.
            ...(files.length === 1 && title.trim() !== '' ? { title: title.trim() } : {}),
            tags: parsedTags,
          }),
        );
      }
      return { uploaded };
    },
    onSuccess: async (produced) => {
      setOutcome(produced);
      setFiles([]);
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
    },
  });

  const { formError, fieldErrors } = partitionUploadError(upload.error);
  const destination =
    categoryId === null
      ? 'Unfiled'
      : (findCategory(categories, categoryId)?.name ?? 'the selected category');

  const uploadedResults = outcome !== null && 'uploaded' in outcome ? outcome.uploaded : null;
  const importResult = outcome !== null && 'imported' in outcome ? outcome.imported : null;
  const duplicates = uploadedResults?.filter((result) => result.duplicate_of !== null) ?? [];

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

      {uploadedResults !== null && uploadedResults.length === 1 && (
        <Callout
          tone={duplicates.length === 0 ? 'success' : 'caution'}
          title={
            duplicates.length === 0
              ? `Uploaded ${uploadedResults[0]?.document.title ?? 'the document'}`
              : 'Uploaded, but this content already exists'
          }
        >
          {duplicates.length === 0
            ? 'Filed as a draft. Submit it for review when it is ready.'
            : 'Another document already has identical content. Both are kept, and the bytes are stored once.'}
        </Callout>
      )}

      {uploadedResults !== null && uploadedResults.length > 1 && (
        <Callout tone="success" title={`Uploaded ${String(uploadedResults.length)} documents`}>
          Each file became its own draft document.
          {duplicates.length > 0 &&
            ` ${String(duplicates.length)} of them duplicate existing content; both copies are kept, and the bytes are stored once.`}
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
          Files
        </label>
        <p className="el-field__hint">
          PDF, images, office documents, or plain text — choose several to upload them all. The
          original is stored byte-for-byte and never modified. A ZIP archive is imported whole:
          its folders become categories and its files become documents.
        </p>
        <input
          id={fileInputId}
          ref={fileInput}
          type="file"
          multiple
          className="el-field__control"
          required
          aria-invalid={fieldErrors.file === undefined ? undefined : true}
          onChange={(event) => {
            setFiles(Array.from(event.target.files ?? []));
          }}
        />
        <div role="alert" aria-live="polite">
          {fieldErrors.file !== undefined && (
            <p className="el-field__error">{fieldErrors.file}</p>
          )}
        </div>
      </div>

      {!zipChosen && (
        <>
          {files.length <= 1 && (
            <TextField
              label="Title"
              hint="Left empty, the filename is used."
              value={title}
              error={fieldErrors.title}
              onChange={(event) => {
                setTitle(event.target.value);
              }}
            />
          )}

          <TextField
            label="Tags"
            hint={
              files.length > 1
                ? 'Comma separated, applied to every file in this upload.'
                : 'Comma separated. Existing tags are reused regardless of capitalisation.'
            }
            value={tags}
            error={fieldErrors.tags}
            onChange={(event) => {
              setTags(event.target.value);
            }}
          />
        </>
      )}

      <p className="el-muted" style={{ fontSize: 'var(--el-text-xs)' }}>
        {zipChosen ? (
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
        disabled={files.length === 0}
        isLoading={upload.isPending}
        loadingLabel={zipChosen ? 'Importing' : 'Uploading'}
      >
        {zipChosen
          ? 'Import archive'
          : files.length > 1
            ? `Upload ${String(files.length)} documents`
            : 'Upload document'}
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
