import { useEffect, useRef, useState, type FormEvent } from "react";
import { Check, Eye, EyeOff, KeyRound, ShieldCheck, X } from "lucide-react";
import { createInitialAdmin } from "../../lib/api";

type SetupDialogProps = {
  open: boolean;
  onClose: () => void;
  onComplete: () => void;
};

export function SetupDialog({ open, onClose, onComplete }: SetupDialogProps) {
  const usernameInput = useRef<HTMLInputElement>(null);
  const [username, setUsername] = useState("admin");
  const [password, setPassword] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [error, setError] = useState<string>();
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (!open) return;
    usernameInput.current?.focus();
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !submitting) onClose();
    };
    document.addEventListener("keydown", closeOnEscape);
    return () => document.removeEventListener("keydown", closeOnEscape);
  }, [onClose, open, submitting]);

  if (!open) return null;

  const passwordLongEnough = password.length >= 12;
  const passwordsMatch = password.length > 0 && password === confirmation;

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setError(undefined);
    if (!passwordsMatch) {
      setError("The password confirmation does not match.");
      return;
    }

    setSubmitting(true);
    try {
      await createInitialAdmin(username, password);
      onComplete();
      onClose();
    } catch (caughtError) {
      setError(caughtError instanceof Error ? caughtError.message : "Setup could not be completed.");
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="dialog-backdrop" role="presentation" onMouseDown={submitting ? undefined : onClose}>
      <section
        aria-labelledby="setup-title"
        aria-modal="true"
        className="setup-dialog"
        onMouseDown={(event) => event.stopPropagation()}
        role="dialog"
      >
        <header className="dialog-header">
          <div className="dialog-symbol"><ShieldCheck size={22} /></div>
          <div>
            <p className="eyebrow">First-run setup</p>
            <h2 id="setup-title">Create the library administrator</h2>
          </div>
          <button aria-label="Close setup" disabled={submitting} onClick={onClose} type="button">
            <X size={19} />
          </button>
        </header>

        <form onSubmit={submit}>
          <p className="dialog-introduction">
            This account stays inside Elrond. No name, email address, or external identity is required.
          </p>

          <label className="form-field">
            <span>Username</span>
            <input
              autoComplete="username"
              maxLength={64}
              minLength={3}
              onChange={(event) => setUsername(event.target.value)}
              pattern="[A-Za-z0-9._-]+"
              ref={usernameInput}
              required
              value={username}
            />
            <small>Letters, numbers, dots, dashes, and underscores.</small>
          </label>

          <label className="form-field">
            <span>Password</span>
            <div className="password-input">
              <KeyRound size={17} aria-hidden="true" />
              <input
                autoComplete="new-password"
                maxLength={128}
                minLength={12}
                onChange={(event) => setPassword(event.target.value)}
                required
                type={showPassword ? "text" : "password"}
                value={password}
              />
              <button
                aria-label={showPassword ? "Hide password" : "Show password"}
                onClick={() => setShowPassword((shown) => !shown)}
                type="button"
              >
                {showPassword ? <EyeOff size={17} /> : <Eye size={17} />}
              </button>
            </div>
          </label>

          <label className="form-field">
            <span>Confirm password</span>
            <input
              autoComplete="new-password"
              maxLength={128}
              onChange={(event) => setConfirmation(event.target.value)}
              required
              type={showPassword ? "text" : "password"}
              value={confirmation}
            />
          </label>

          <div className="password-checks" aria-live="polite">
            <span className={passwordLongEnough ? "met" : undefined}>
              <Check size={14} /> At least 12 characters
            </span>
            <span className={passwordsMatch ? "met" : undefined}>
              <Check size={14} /> Confirmation matches
            </span>
          </div>

          {error && <p className="form-error" role="alert">{error}</p>}

          <footer className="dialog-actions">
            <button className="dialog-cancel" disabled={submitting} onClick={onClose} type="button">Not yet</button>
            <button
              className="dialog-submit"
              disabled={submitting || !passwordLongEnough || !passwordsMatch}
              type="submit"
            >
              {submitting ? "Securing library..." : "Create administrator"}
            </button>
          </footer>
        </form>
      </section>
    </div>
  );
}
