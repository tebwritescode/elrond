import { EmptyState, Panel } from '@/components';
import { PageHeader } from '@/components/PageHeader';

export interface ComingSoonProps {
  readonly eyebrow: string;
  readonly title: string;
  readonly lede: string;
  readonly milestone: string;
  /** What will be built here, so the route is informative rather than empty. */
  readonly plan: readonly string[];
}

/**
 * A route that exists in the navigation but whose feature has not landed.
 *
 * Preferred over hiding the route: the shape of the product is visible from the
 * first milestone, and the page says plainly what is coming and when instead of
 * looking broken.
 */
export function ComingSoonPage({ eyebrow, title, lede, milestone, plan }: ComingSoonProps) {
  return (
    <div className="el-stack">
      <PageHeader eyebrow={eyebrow} title={title} lede={lede} />
      <Panel title={`Planned for ${milestone}`}>
        <EmptyState title="Not built yet">
          <span>This area is reserved. It will provide:</span>
        </EmptyState>
        <ul
          className="el-measure"
          style={{
            paddingLeft: 'var(--el-space-5)',
            color: 'var(--el-ink-muted)',
            fontSize: 'var(--el-text-sm)',
          }}
        >
          {plan.map((item) => (
            <li key={item} style={{ marginBottom: 'var(--el-space-2)' }}>
              {item}
            </li>
          ))}
        </ul>
      </Panel>
    </div>
  );
}

/** The documents library route. */
export function DocumentsPage() {
  return (
    <ComingSoonPage
      eyebrow="Library"
      title="Documents"
      lede="Originals stay byte-for-byte immutable; PDF is the canonical format for viewing, editing, and distribution."
      milestone="v0.2.0"
      plan={[
        'Upload with SHA-256 content checksums and duplicate detection.',
        'A persistent hierarchical category tree alongside a full-width sortable table.',
        'Full-text search over metadata and extracted content, backed by SQLite FTS5.',
        'ZIP hierarchy import that turns folders into categories, with archive-bomb and path-traversal rejection.',
        'Generated PDF copies for office files and images, keeping the source file intact.',
      ]}
    />
  );
}

/** The binder designer route. */
export function BindersPage() {
  return (
    <ComingSoonPage
      eyebrow="Publishing"
      title="Binders"
      lede="A binder is an ordered tree of sections and documents. Each release pins published version identifiers so the output can be reproduced exactly."
      milestone="v0.4.0"
      plan={[
        'A three-pane designer: source library, outline, and properties with live preview.',
        'Drag-and-drop ordering with equivalent keyboard movement controls.',
        'Generated covers, separator pages, a clickable table of contents, and indexes.',
        'PDF bookmarks, page labels, headers and footers, and optional duplex blank pages.',
        'Release history with rebuild and compare-before-release.',
      ]}
    />
  );
}
