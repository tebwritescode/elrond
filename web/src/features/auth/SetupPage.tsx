import { useState } from 'react';

import { Button, Callout, TextField, Wordmark } from '@/components';

import { partitionError, useCompleteSetup } from './session';

/** Mirrors `PasswordPolicy::MIN_LENGTH` in the domain crate. */
const PASSWORD_MIN_LENGTH = 12;

/**
 * First-run setup.
 *
 * Shown when the instance has no accounts. The server closes this endpoint
 * permanently once one exists, so there is no window in which a second
 * administrator could be created here.
 */
export function SetupPage({ version }: { readonly version: string }) {
  const setup = useCompleteSetup();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [confirmation, setConfirmation] = useState('');
  const [mismatch, setMismatch] = useState<string | undefined>(undefined);

  const { formError, fieldErrors } = partitionError(setup.error);

  function submit() {
    // Checked here rather than on the server: the confirmation field exists only
    // to catch a typo in this browser, so there is nothing for the API to know
    // about it.
    if (password !== confirmation) {
      setMismatch('The two passwords do not match.');
      return;
    }
    setMismatch(undefined);
    setup.mutate({ username, password });
  }

  return (
    <main className="el-gate">
      <div className="el-gate__card">
        <Wordmark version={version} />
        <h1 style={{ marginTop: 'var(--el-space-4)' }}>Set up your library</h1>
        <p className="el-gate__lede">
          This instance has no accounts yet. Choose a username and password for the
          administrator who will own it. Once this account exists, this screen is closed for
          good and further accounts are created from the administration area.
        </p>

        {/* The handler takes no parameter, so the event type never has to be
            named; React infers it here. */}
        <form
          className="el-form"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            submit();
          }}
        >
          {formError !== undefined && (
            <Callout tone="danger" title="Could not complete setup">
              {formError}
            </Callout>
          )}

          <TextField
            label="Username"
            name="username"
            autoComplete="username"
            required
            autoFocus
            spellCheck={false}
            autoCapitalize="none"
            hint="Letters, digits, dots, underscores, and hyphens. Stored in lowercase."
            value={username}
            error={fieldErrors.username}
            onChange={(event) => {
              setUsername(event.target.value);
            }}
          />

          <TextField
            label="Password"
            type="password"
            name="password"
            autoComplete="new-password"
            required
            hint={`At least ${String(PASSWORD_MIN_LENGTH)} characters. A memorable phrase beats a short, complicated word.`}
            value={password}
            error={fieldErrors.password}
            onChange={(event) => {
              setPassword(event.target.value);
            }}
          />

          <TextField
            label="Confirm password"
            type="password"
            name="passwordConfirmation"
            autoComplete="new-password"
            required
            value={confirmation}
            error={mismatch}
            onChange={(event) => {
              setConfirmation(event.target.value);
              if (mismatch !== undefined) {
                setMismatch(undefined);
              }
            }}
          />

          <Button
            type="submit"
            variant="primary"
            size="lg"
            block
            isLoading={setup.isPending}
            loadingLabel="Creating the administrator"
          >
            Create administrator
          </Button>
        </form>
      </div>
    </main>
  );
}
