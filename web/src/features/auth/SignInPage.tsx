import { useState } from 'react';

import { Button, Callout, TextField, Wordmark } from '@/components';
import { ApiError } from '@/lib/api';

import { partitionError, useSignIn } from './session';

/** Sign-in screen, shown whenever there is no valid session. */
export function SignInPage({ version }: { readonly version: string }) {
  const signIn = useSignIn();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');

  const { formError, fieldErrors } = partitionError(signIn.error);
  const throttled = signIn.error instanceof ApiError && signIn.error.code === 'rate_limited';

  return (
    <main className="el-gate">
      <div className="el-gate__card">
        <Wordmark version={version} />
        <h1 style={{ marginTop: 'var(--el-space-4)' }}>Sign in</h1>
        <p className="el-gate__lede">Enter your credentials to reach the document library.</p>

        <form
          className="el-form"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            signIn.mutate({ username, password });
          }}
        >
          {formError !== undefined && (
            <Callout
              tone={throttled ? 'caution' : 'danger'}
              title={throttled ? 'Too many attempts' : 'Could not sign in'}
            >
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
            autoComplete="current-password"
            required
            value={password}
            error={fieldErrors.password}
            onChange={(event) => {
              setPassword(event.target.value);
            }}
          />

          <Button
            type="submit"
            variant="primary"
            size="lg"
            block
            isLoading={signIn.isPending}
            loadingLabel="Signing in"
          >
            Sign in
          </Button>
        </form>
      </div>
    </main>
  );
}
