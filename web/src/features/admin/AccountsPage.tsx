import { useQuery } from '@tanstack/react-query';

import { Callout, EmptyState, Panel, Pill, Skeleton } from '@/components';
import { PageHeader } from '@/components/PageHeader';
import { ApiError, api, type Role, type UserView } from '@/lib/api';

/** Tone used for each role's pill. Accent marks the privileged one. */
const ROLE_TONE: Readonly<Record<Role, 'neutral' | 'accent'>> = {
  viewer: 'neutral',
  reviewer: 'neutral',
  editor: 'neutral',
  admin: 'accent',
};

/** Formats an RFC 3339 timestamp in the viewer's locale. */
function formatDate(iso: string): string {
  const parsed = new Date(iso);
  if (Number.isNaN(parsed.getTime())) {
    return iso;
  }
  return parsed.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}

/** Administrator view of local accounts. */
export function AccountsPage() {
  const accounts = useQuery({
    queryKey: ['users'],
    queryFn: () => api.listUsers(),
    // A 403 means this account is not an administrator; retrying cannot help.
    retry: (failureCount, error) =>
      !(error instanceof ApiError && error.status < 500) && failureCount < 2,
  });

  return (
    <div className="el-stack">
      <PageHeader
        eyebrow="Administration"
        title="Accounts"
        lede="Local accounts and their authority. Roles form a ladder: an editor can do everything a reviewer can, and an administrator everything an editor can."
      />

      {accounts.isError && (
        <Callout tone="danger" title="Could not load accounts">
          {accounts.error.message}
        </Callout>
      )}

      <Panel title="Local accounts" flush>
        {accounts.isPending ? (
          <div
            className="el-stack"
            style={{ gap: 'var(--el-space-3)', padding: 'var(--el-space-5)' }}
          >
            {/* Skeletons sized to the rows they replace, so the layout does not shift. */}
            <Skeleton height="1.25rem" width="40%" />
            <Skeleton height="1.25rem" width="65%" />
            <Skeleton height="1.25rem" width="52%" />
          </div>
        ) : (
          <AccountsTable accounts={accounts.data ?? []} />
        )}
      </Panel>
    </div>
  );
}

/** The accounts table, or an explanation of why it is empty. */
function AccountsTable({ accounts }: { readonly accounts: readonly UserView[] }) {
  if (accounts.length === 0) {
    return (
      <EmptyState title="No accounts to show">
        This is unexpected: at least the administrator created during setup should appear here.
        If the list is genuinely empty, the database may have been replaced underneath the
        running server.
      </EmptyState>
    );
  }

  return (
    <div className="el-table-wrap">
      <table className="el-table">
        <caption className="el-visually-hidden">
          Local accounts, oldest first, with role and status
        </caption>
        {/* No email column: the model has no contact details to show. */}
        <thead>
          <tr>
            <th scope="col">Username</th>
            <th scope="col">Role</th>
            <th scope="col">Status</th>
            <th scope="col">Created</th>
          </tr>
        </thead>
        <tbody>
          {accounts.map((account) => (
            <tr key={account.id}>
              {/* The username is the row header, so a screen reader announces it
                  alongside each cell instead of reading bare values. */}
              <th
                scope="row"
                style={{
                  fontWeight: 550,
                  textTransform: 'none',
                  letterSpacing: 0,
                  fontSize: 'var(--el-text-sm)',
                  color: 'var(--el-ink)',
                }}
              >
                {account.username}
              </th>
              <td>
                <Pill tone={ROLE_TONE[account.role]}>{account.role}</Pill>
              </td>
              <td>
                {/* Text, not a coloured dot: status must not be conveyed by colour alone. */}
                <Pill tone={account.is_active ? 'success' : 'caution'}>
                  {account.is_active ? 'Active' : 'Deactivated'}
                </Pill>
              </td>
              <td>{formatDate(account.created_at)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
