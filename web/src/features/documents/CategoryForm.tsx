import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';

import { Button, Callout, TextField } from '@/components';
import { api, type CategoryNode } from '@/lib/api';

import { partitionUploadError } from './queries';

export interface CategoryFormProps {
  /**
   * Category the new one is nested under, or null to create a root category.
   * This is whatever the tree has selected, so "new category" always means
   * "here", which is where the user is already looking.
   */
  readonly parentId: string | null;
  readonly categories: readonly CategoryNode[];
}

/**
 * Creates a category.
 *
 * Collapsed behind a disclosure rather than sitting open above the tree: the
 * tree is the primary control in this sidebar and categories are created rarely,
 * so the form should not compete with it for the top of the panel.
 */
export function CategoryForm({ parentId, categories }: CategoryFormProps) {
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const [name, setName] = useState('');

  const create = useMutation({
    mutationFn: () => {
      const trimmed = name.trim();
      if (trimmed === '') {
        throw new Error('Give the category a name.');
      }
      return parentId === null
        ? api.createCategory(trimmed)
        : api.createCategory(trimmed, parentId);
    },
    onSuccess: async () => {
      setName('');
      setOpen(false);
      await queryClient.invalidateQueries({ queryKey: ['categories'] });
    },
  });

  const { formError, fieldErrors } = partitionUploadError(create.error);
  const parentName =
    parentId === null ? null : (findCategory(categories, parentId)?.name ?? null);

  if (!open) {
    return (
      <Button
        variant="secondary"
        onClick={() => {
          setOpen(true);
        }}
        aria-expanded={false}
      >
        New category
      </Button>
    );
  }

  return (
    <form
      className="el-stack"
      style={{ gap: 'var(--el-space-3)' }}
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        create.mutate();
      }}
    >
      {formError !== undefined && (
        <Callout tone="danger" title="Could not create the category">
          {formError}
        </Callout>
      )}

      <TextField
        label="New category"
        hint={
          parentName === null
            ? 'Created at the top level. Select a category first to nest one inside it.'
            : `Created inside ${parentName}.`
        }
        value={name}
        error={fieldErrors.name}
        onChange={(event) => {
          setName(event.target.value);
        }}
        autoFocus
      />

      <div style={{ display: 'flex', gap: 'var(--el-space-2)' }}>
        <Button type="submit" variant="primary" disabled={create.isPending}>
          {create.isPending ? 'Creating…' : 'Create'}
        </Button>
        <Button
          variant="ghost"
          onClick={() => {
            setOpen(false);
            setName('');
            create.reset();
          }}
        >
          Cancel
        </Button>
      </div>
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
