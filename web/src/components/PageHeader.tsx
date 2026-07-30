import type { ReactNode } from 'react';

export interface PageHeaderProps {
  readonly eyebrow?: string;
  readonly title: string;
  readonly lede?: string;
  readonly action?: ReactNode;
}

/**
 * The heading block every page opens with.
 *
 * Renders the single `<h1>` for the page, so heading order is correct without
 * each page having to remember.
 */
export function PageHeader({ eyebrow, title, lede, action }: PageHeaderProps) {
  return (
    <header className="el-page-header">
      <div>
        {eyebrow !== undefined && <p className="el-eyebrow">{eyebrow}</p>}
        <h1>{title}</h1>
        {lede !== undefined && <p className="el-page-header__lede">{lede}</p>}
      </div>
      {action}
    </header>
  );
}
