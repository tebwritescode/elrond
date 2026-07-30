import { Button, EmptyState, Panel } from '@/components';
import { PageHeader } from '@/components/PageHeader';

/**
 * Shown for a URL that matches no route.
 *
 * In its own file so `router.tsx` exports only the router, which is what lets
 * Vite's fast refresh keep working on the route tree.
 */
export function NotFoundPage({ onGoHome }: { readonly onGoHome: () => void }) {
  return (
    <div className="el-stack">
      <PageHeader eyebrow="Error 404" title="That page does not exist" />
      <Panel>
        <EmptyState
          title="Nothing here"
          action={
            <Button variant="secondary" onClick={onGoHome}>
              Back to the dashboard
            </Button>
          }
        >
          The address may be mistyped, or it may point at something that has since been
          archived.
        </EmptyState>
      </Panel>
    </div>
  );
}
