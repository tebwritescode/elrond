import type { ReactNode } from 'react';

export interface PanelProps {
  readonly title?: string;
  readonly action?: ReactNode;
  readonly children: ReactNode;
  /** Removes the body padding, for panels wrapping a full-bleed table. */
  readonly flush?: boolean;
}

/** A bordered surface with an optional header. */
export function Panel({ title, action, children, flush = false }: PanelProps) {
  return (
    <section className="el-panel">
      {title !== undefined && (
        <header className="el-panel__header">
          <h2>{title}</h2>
          {action}
        </header>
      )}
      <div className={flush ? '' : 'el-panel__body'}>{children}</div>
    </section>
  );
}

/** A status pill. Always renders a word, never a bare colour. */
export function Pill({
  tone = 'neutral',
  children,
}: {
  readonly tone?: 'neutral' | 'accent' | 'success' | 'caution';
  readonly children: ReactNode;
}) {
  return <span className={`el-pill el-pill--${tone}`}>{children}</span>;
}

/**
 * A placeholder for a collection with nothing in it.
 *
 * Always states what would appear here and what to do next, rather than showing
 * a bare "No results".
 */
export function EmptyState({
  title,
  children,
  action,
}: {
  readonly title: string;
  readonly children: ReactNode;
  readonly action?: ReactNode;
}) {
  return (
    <div className="el-empty">
      <p className="el-empty__title">{title}</p>
      <p className="el-muted el-measure">{children}</p>
      {action}
    </div>
  );
}

/** A loading placeholder sized to the content it stands in for. */
export function Skeleton({
  width = '100%',
  height = '1rem',
}: {
  readonly width?: string;
  readonly height?: string;
}) {
  // aria-hidden because the surrounding region already announces that it is busy.
  return (
    <span
      className="el-skeleton"
      style={{ width, height, display: 'block' }}
      aria-hidden="true"
    />
  );
}
