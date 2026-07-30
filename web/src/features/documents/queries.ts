import { useQuery, type UseQueryResult } from '@tanstack/react-query';

import {
  ApiError,
  api,
  type CategoryNode,
  type DocumentDetail,
  type DocumentPage,
  type DocumentQuery,
  type TagCount,
} from '@/lib/api';

/** Reads the category tree. */
export function useCategories(): UseQueryResult<CategoryNode[]> {
  return useQuery({
    queryKey: ['categories'],
    queryFn: () => api.categories(),
    staleTime: 30_000,
  });
}

/** Reads every tag with its document count. */
export function useTags(): UseQueryResult<TagCount[]> {
  return useQuery({
    queryKey: ['tags'],
    queryFn: () => api.tags(),
    staleTime: 30_000,
  });
}

/**
 * Reads a page of documents.
 *
 * The query object is part of the key, so changing a filter is a distinct cache
 * entry and going back to a previous filter is instant.
 */
export function useDocuments(query: DocumentQuery): UseQueryResult<DocumentPage> {
  return useQuery({
    queryKey: ['documents', query],
    queryFn: () => api.documents(query),
    // Keeps the previous page visible while the next one loads, so the table does
    // not collapse to a spinner on every keystroke of a search.
    placeholderData: (previous) => previous,
  });
}

/** Reads one document with its version history. */
export function useDocument(id: string | null): UseQueryResult<DocumentDetail> {
  return useQuery({
    queryKey: ['document', id],
    queryFn: () => {
      if (id === null) {
        throw new Error('no document selected');
      }
      return api.document(id);
    },
    enabled: id !== null,
  });
}

/**
 * Splits an upload failure into field-level and form-level messages.
 *
 * The server names the offending field for validation errors, so the message can
 * sit next to the input it concerns rather than in a summary the user has to
 * match up themselves.
 */
export function partitionUploadError(error: unknown): {
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

/** Formats a byte count for display. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) {
    return `${String(bytes)} B`;
  }
  const units = ['kB', 'MB', 'GB', 'TB'];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  // One decimal below 10, none above: "1.4 MB" is useful, "1.4 kB" less so than
  // "1.4", and "234.7 MB" is false precision.
  return `${value < 10 ? value.toFixed(1) : Math.round(value).toString()} ${units[unit] ?? 'B'}`;
}

/** Formats an RFC 3339 timestamp in the viewer's locale. */
export function formatDate(iso: string): string {
  const parsed = new Date(iso);
  return Number.isNaN(parsed.getTime())
    ? iso
    : parsed.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}

/** Human-readable label for a lifecycle state. */
export const LIFECYCLE_LABELS: Readonly<Record<string, string>> = {
  draft: 'Draft',
  in_review: 'In review',
  published: 'Published',
  superseded: 'Superseded',
  archived: 'Archived',
};

/** Pill tone for a lifecycle state. Never the only signal; the label is shown too. */
export const LIFECYCLE_TONES: Readonly<
  Record<string, 'neutral' | 'accent' | 'success' | 'caution'>
> = {
  draft: 'neutral',
  in_review: 'caution',
  published: 'success',
  superseded: 'neutral',
  archived: 'neutral',
};
