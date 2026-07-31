import { Button, Callout, Panel, Pill, Skeleton } from '@/components';
import { originalUrl, pdfUrl } from '@/lib/api';

import {
  LIFECYCLE_LABELS,
  LIFECYCLE_TONES,
  formatBytes,
  formatDate,
  useDocument,
} from './queries';

export interface DocumentInfoPanelProps {
  /** Document to describe. */
  readonly documentId: string;
  readonly onClose: () => void;
}

/**
 * Everything known about one document: metadata, provenance, and the full
 * version history.
 *
 * Opened from the ⓘ button in the table. Reading a document and reading *about*
 * it are different intents — clicking a row opens the PDF itself, and this panel
 * carries the rest.
 */
export function DocumentInfoPanel({ documentId, onClose }: DocumentInfoPanelProps) {
  const detail = useDocument(documentId);

  return (
    <Panel
      title="Document details"
      action={
        <Button variant="ghost" size="sm" onClick={onClose}>
          Close
        </Button>
      }
    >
      {detail.isPending && (
        <div className="el-stack" style={{ gap: 'var(--el-space-2)' }}>
          <Skeleton height="1.5rem" width="50%" />
          <Skeleton height="1rem" />
          <Skeleton height="1rem" width="80%" />
        </div>
      )}

      {detail.isError && (
        <Callout tone="danger" title="Could not load the document">
          {detail.error instanceof Error ? detail.error.message : 'Please try again.'}
        </Callout>
      )}

      {detail.data !== undefined && (
        <div className="el-stack" style={{ gap: 'var(--el-space-3)' }}>
          <div>
            <h3 style={{ margin: 0 }}>{detail.data.title}</h3>
            <p className="el-muted" style={{ margin: 'var(--el-space-1) 0 0' }}>
              {detail.data.current_version.filename}
            </p>
          </div>

          <dl className="el-info-grid">
            <dt>Category</dt>
            <dd>{detail.data.category_name}</dd>
            <dt>State</dt>
            <dd>
              <Pill tone={LIFECYCLE_TONES[detail.data.lifecycle] ?? 'neutral'}>
                {LIFECYCLE_LABELS[detail.data.lifecycle] ?? detail.data.lifecycle}
              </Pill>
            </dd>
            <dt>Type</dt>
            <dd>{detail.data.current_version.media_type}</dd>
            <dt>Size</dt>
            <dd>{formatBytes(detail.data.current_version.byte_size)}</dd>
            <dt>Added</dt>
            <dd>{formatDate(detail.data.created_at)}</dd>
            <dt>Changed</dt>
            <dd>{formatDate(detail.data.updated_at)}</dd>
            {detail.data.source_path !== null && (
              <>
                <dt>Imported from</dt>
                <dd>{detail.data.source_path}</dd>
              </>
            )}
            {detail.data.tags.length > 0 && (
              <>
                <dt>Tags</dt>
                <dd>
                  <span
                    className="el-row"
                    style={{ gap: 'var(--el-space-1)', flexWrap: 'wrap' }}
                  >
                    {detail.data.tags.map((tag) => (
                      <Pill key={tag.id}>{tag.label}</Pill>
                    ))}
                  </span>
                </dd>
              </>
            )}
          </dl>

          <div className="el-row" style={{ gap: 'var(--el-space-2)' }}>
            {detail.data.current_version.has_pdf && (
              <a
                className="el-button el-button--primary el-button--sm"
                href={pdfUrl(detail.data.current_version.id)}
                target="_blank"
                rel="noreferrer"
              >
                Open document
              </a>
            )}
            <a
              className="el-button el-button--secondary el-button--sm"
              href={originalUrl(detail.data.current_version.id)}
            >
              Download original
            </a>
          </div>

          <section>
            <h4 style={{ margin: '0 0 var(--el-space-2)' }}>
              {detail.data.versions.length === 1
                ? '1 version'
                : `${String(detail.data.versions.length)} versions`}
            </h4>
            <ul
              className="el-stack"
              style={{ gap: 'var(--el-space-1)', margin: 0, padding: 0 }}
            >
              {detail.data.versions.map((version) => (
                <li key={version.id} className="el-row" style={{ gap: 'var(--el-space-2)' }}>
                  <span>
                    v{version.number} · {version.filename} · {formatBytes(version.byte_size)} ·{' '}
                    {formatDate(version.created_at)}
                  </span>
                  <a href={originalUrl(version.id)}>Download</a>
                </li>
              ))}
            </ul>
          </section>
        </div>
      )}
    </Panel>
  );
}
