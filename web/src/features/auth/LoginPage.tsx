import { useState, type FormEvent } from "react";
import { ArrowRight, BookOpen, FileStack, KeyRound, Library, ShieldCheck } from "lucide-react";
import { login } from "../../lib/api";

type LoginPageProps = {
  onLogin: () => void;
};

export function LoginPage({ onLogin }: LoginPageProps) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string>();
  const [submitting, setSubmitting] = useState(false);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setError(undefined);
    setSubmitting(true);
    try {
      await login(username, password);
      onLogin();
    } catch (caughtError) {
      setError(caughtError instanceof Error ? caughtError.message : "Sign in failed.");
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <main className="login-page">
      <section className="login-story" aria-label="About Elrond">
        <div className="login-wordmark">
          <span><FileStack size={22} /></span>
          <div><strong>ELROND</strong><small>DOCUMENT LIBRARY</small></div>
        </div>
        <div className="login-message">
          <p className="eyebrow">Controlled knowledge</p>
          <h1>One library.<br />Every trusted version.</h1>
          <p>Preserve source documents, govern changes, and publish organized binders without losing their history.</p>
        </div>
        <div className="login-capabilities">
          <span><Library size={17} /> Structured library</span>
          <span><ShieldCheck size={17} /> Local and private</span>
          <span><BookOpen size={17} /> Reproducible binders</span>
        </div>
      </section>

      <section className="login-form-area">
        <form className="login-form" onSubmit={submit}>
          <div className="login-form-heading">
            <p className="eyebrow">Welcome back</p>
            <h2>Open your library</h2>
            <p>Sign in with your local Elrond account.</p>
          </div>
          <label className="form-field">
            <span>Username</span>
            <input
              autoComplete="username"
              autoFocus
              onChange={(event) => setUsername(event.target.value)}
              required
              value={username}
            />
          </label>
          <label className="form-field">
            <span>Password</span>
            <div className="password-input">
              <KeyRound size={17} aria-hidden="true" />
              <input
                autoComplete="current-password"
                onChange={(event) => setPassword(event.target.value)}
                required
                type="password"
                value={password}
              />
            </div>
          </label>
          {error && <p className="form-error" role="alert">{error}</p>}
          <button className="login-submit" disabled={submitting} type="submit">
            {submitting ? "Opening library..." : "Open library"}
            {!submitting && <ArrowRight size={17} />}
          </button>
          <p className="login-privacy">Credentials are verified by this Elrond instance and are never sent to an external identity service.</p>
        </form>
      </section>
    </main>
  );
}
