import { EmptyState, Panel, Pill } from '@/components';
import { PageHeader } from '@/components/PageHeader';
import { useBootstrap } from '@/features/auth/session';

/** A queue of work shown on the dashboard. */
interface Queue {
  readonly title: string;
  readonly description: string;
  /** Replaced by real counts once documents exist. */
  readonly milestone: string;
}

/**
 * The task queues the dashboard will surface.
 *
 * Listed now, with their state honestly labelled, rather than hidden until the
 * feature lands: the shape of the workspace is the thing being reviewed at this
 * milestone.
 */
const QUEUES: readonly Queue[] = [
  {
    title: 'Drafts',
    description:
      'Documents you are still preparing. Editable in place until submitted for review.',
    milestone: 'v0.2.0',
  },
  {
    title: 'Awaiting review',
    description:
      'Submitted documents frozen until a reviewer approves them or asks for changes.',
    milestone: 'v0.3.0',
  },
  {
    title: 'Expiring soon',
    description: 'Published documents approaching their review date.',
    milestone: 'v0.3.0',
  },
  {
    title: 'Failed jobs',
    description:
      'Conversions, OCR runs, and binder builds that did not finish, with the reason and a way to retry.',
    milestone: 'v0.4.0',
  },
];

/** Task-oriented landing page. */
export function DashboardPage() {
  const bootstrap = useBootstrap();
  const user = bootstrap.data?.user ?? null;

  return (
    <div className="el-stack">
      <PageHeader
        eyebrow="Overview"
        title={user === null ? 'Dashboard' : `Welcome back, ${user.username}`}
        lede="Elrond opens on the work waiting for you rather than on a summary of the archive. Each queue below becomes live as its milestone lands."
      />

      <div className="el-grid-cards">
        {QUEUES.map((queue) => (
          <Panel key={queue.title}>
            <div className="el-stack" style={{ gap: 'var(--el-space-3)' }}>
              <div className="el-row" style={{ justifyContent: 'space-between' }}>
                <h2 style={{ fontSize: 'var(--el-text-md)' }}>{queue.title}</h2>
                <Pill tone="neutral">{queue.milestone}</Pill>
              </div>
              <p className="el-muted" style={{ fontSize: 'var(--el-text-sm)' }}>
                {queue.description}
              </p>
            </div>
          </Panel>
        ))}
      </div>

      <Panel title="Recent activity">
        <EmptyState title="Nothing has happened yet">
          Once documents are ingested, every change appends an audit record and the most recent
          ones appear here. The audit table is append-only in the database itself, so this
          history cannot be rewritten.
        </EmptyState>
      </Panel>
    </div>
  );
}
