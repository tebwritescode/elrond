import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';

import { Button, Callout, TextField } from '@/components';
import { api, type CategoryNode } from '@/lib/api';

export interface CategoryManagerProps {
  /** Category to manage — whatever the tree has selected. */
  readonly categoryId: string;
  readonly categories: readonly CategoryNode[];
  /** Called after a successful delete, so the page can drop its selection. */
  readonly onDeleted: () => void;
}

/**
 * Rename and delete for the selected category.
 *
 * The tree is the selector: managing "the selected category" keeps the controls
 * next to the thing they act on, instead of duplicating the whole hierarchy
 * into a second management screen.
 *
 * Render with `key={categoryId}`: remounting on selection change is what
 * abandons an in-progress rename, so a half-typed name can never be applied to
 * the wrong category.
 */
export function CategoryManager({ categoryId, categories, onDeleted }: CategoryManagerProps) {
  const queryClient = useQueryClient();
  const category = findCategory(categories, categoryId);
  const [renaming, setRenaming] = useState(false);
  const [name, setName] = useState('');

  const rename = useMutation({
    mutationFn: () => {
      const trimmed = name.trim();
      if (trimmed === '') {
        throw new Error('Give the category a name.');
      }
      return api.renameCategory(categoryId, trimmed);
    },
    onSuccess: async () => {
      setRenaming(false);
      setName('');
      await queryClient.invalidateQueries({ queryKey: ['categories'] });
      await queryClient.invalidateQueries({ queryKey: ['documents'] });
    },
  });

  const remove = useMutation({
    mutationFn: () => api.deleteCategory(categoryId),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['categories'] });
      onDeleted();
    },
  });

  if (category === undefined) {
    return null;
  }

  const error = rename.error ?? remove.error;

  return (
    <div className="el-stack" style={{ gap: 'var(--el-space-2)' }}>
      {error !== null && (
        <Callout tone="danger" title="Could not change the category">
          {error instanceof Error ? error.message : 'Please try again.'}
        </Callout>
      )}

      {renaming ? (
        <form
          className="el-stack"
          style={{ gap: 'var(--el-space-2)' }}
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            rename.mutate();
          }}
        >
          <TextField
            label={`Rename ${category.name}`}
            value={name}
            onChange={(event) => {
              setName(event.target.value);
            }}
            autoFocus
          />
          <div className="el-row" style={{ gap: 'var(--el-space-2)' }}>
            <Button type="submit" size="sm" variant="primary" disabled={rename.isPending}>
              {rename.isPending ? 'Renaming…' : 'Rename'}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                setRenaming(false);
                setName('');
                rename.reset();
              }}
            >
              Cancel
            </Button>
          </div>
        </form>
      ) : (
        <div className="el-row" style={{ gap: 'var(--el-space-2)', flexWrap: 'wrap' }}>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => {
              setRenaming(true);
              setName(category.name);
            }}
          >
            Rename category
          </Button>
          <Button
            size="sm"
            variant="danger"
            isLoading={remove.isPending}
            loadingLabel="Deleting"
            onClick={() => {
              remove.mutate();
            }}
          >
            Delete category
          </Button>
        </div>
      )}

      <p className="el-muted" style={{ fontSize: 'var(--el-text-xs)', margin: 0 }}>
        Acting on <strong>{category.name}</strong>. Deleting is refused while the category still
        holds documents or subcategories.
      </p>
    </div>
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
