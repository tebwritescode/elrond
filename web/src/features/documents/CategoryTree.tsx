import { useState } from 'react';

import type { CategoryNode } from '@/lib/api';

export interface CategoryTreeProps {
  readonly categories: readonly CategoryNode[];
  /** Currently filtered category, or null for the whole library. */
  readonly selectedId: string | null;
  readonly onSelect: (id: string | null) => void;
  /** Total documents in the library, shown against "All documents". */
  readonly total: number;
}

/**
 * The persistent category tree.
 *
 * Built as a nested `<ul>` with `aria-expanded` on the disclosure buttons rather
 * than as an ARIA `tree` widget. A real tree role obliges full arrow-key
 * navigation with a single tab stop, and getting that half-right is worse for a
 * screen reader than a plain nested list, which is already navigable and
 * announces its structure correctly.
 */
export function CategoryTree({ categories, selectedId, onSelect, total }: CategoryTreeProps) {
  return (
    <nav aria-label="Categories">
      <ul className="el-tree">
        <li>
          <button
            type="button"
            className="el-tree__item"
            aria-current={selectedId === null ? 'true' : undefined}
            onClick={() => {
              onSelect(null);
            }}
          >
            <span className="el-tree__label">All documents</span>
            <span className="el-tree__count">{total}</span>
          </button>
        </li>
        {categories.map((category) => (
          <CategoryBranch
            key={category.id}
            category={category}
            depth={0}
            selectedId={selectedId}
            onSelect={onSelect}
          />
        ))}
      </ul>
    </nav>
  );
}

/** One category and, when expanded, its children. */
function CategoryBranch({
  category,
  depth,
  selectedId,
  onSelect,
}: {
  readonly category: CategoryNode;
  readonly depth: number;
  readonly selectedId: string | null;
  readonly onSelect: (id: string | null) => void;
}) {
  // Expanded by default only near the root: a deep tree opened fully would bury
  // the rest of the list.
  const [expanded, setExpanded] = useState(depth < 1);
  const hasChildren = category.children.length > 0;

  return (
    <li>
      <div className="el-tree__row" style={{ paddingLeft: `${String(depth * 0.875)}rem` }}>
        {hasChildren ? (
          <button
            type="button"
            className="el-tree__twisty"
            aria-expanded={expanded}
            // Names the target so a screen reader user knows what will expand,
            // rather than hearing a row of identical "expand" buttons.
            aria-label={`${expanded ? 'Collapse' : 'Expand'} ${category.name}`}
            onClick={() => {
              setExpanded((current) => !current);
            }}
          >
            <svg
              width="10"
              height="10"
              viewBox="0 0 10 10"
              aria-hidden="true"
              focusable="false"
            >
              <path
                d={expanded ? 'M1 3l4 4 4-4' : 'M3 1l4 4-4 4'}
                fill="none"
                stroke="currentColor"
                strokeWidth="1.6"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </button>
        ) : (
          <span className="el-tree__twisty" aria-hidden="true" />
        )}

        <button
          type="button"
          className="el-tree__item"
          aria-current={selectedId === category.id ? 'true' : undefined}
          onClick={() => {
            onSelect(category.id);
          }}
        >
          <span className="el-tree__label">{category.name}</span>
          {/*
            The rolled-up count is shown, since selecting a category includes its
            descendants. The title explains the difference when they diverge.
          */}
          <span
            className="el-tree__count"
            title={
              category.total_document_count === category.document_count
                ? undefined
                : `${String(category.document_count)} directly, ${String(category.total_document_count)} including subcategories`
            }
          >
            {category.total_document_count}
          </span>
        </button>
      </div>

      {hasChildren && expanded && (
        <ul className="el-tree">
          {category.children.map((child) => (
            <CategoryBranch
              key={child.id}
              category={child}
              depth={depth + 1}
              selectedId={selectedId}
              onSelect={onSelect}
            />
          ))}
        </ul>
      )}
    </li>
  );
}
