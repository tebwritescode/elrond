import { useId, type InputHTMLAttributes } from 'react';

import { IconError } from './Icon';

export interface TextFieldProps extends Omit<
  InputHTMLAttributes<HTMLInputElement>,
  'className' | 'id' | 'aria-invalid'
> {
  /** Always-visible label. Placeholders are never used as labels. */
  readonly label: string;
  /** Supporting guidance, wired up with `aria-describedby`. */
  readonly hint?: string;
  /** Validation message. Its presence marks the control invalid. */
  readonly error?: string | undefined;
  /**
   * Whether the field must be filled in.
   *
   * The requirement is stated in the label text as well as in `aria-required`,
   * because a lone asterisk is not a reliable signal.
   */
  readonly required?: boolean;
}

/**
 * A labelled text input.
 *
 * Handles the association plumbing that is easy to get subtly wrong: a real
 * `<label for>`, `aria-describedby` covering both hint and error, `aria-invalid`
 * driven by the error, and an error message announced via a live region so it is
 * heard even when focus has already moved on.
 */
export function TextField({
  label,
  hint,
  error,
  required = false,
  type = 'text',
  ...props
}: TextFieldProps) {
  const id = useId();
  const hintId = `${id}-hint`;
  const errorId = `${id}-error`;

  const describedBy = [hint !== undefined ? hintId : null, error !== undefined ? errorId : null]
    .filter(Boolean)
    .join(' ');

  return (
    <div className="el-field">
      <label className="el-field__label" htmlFor={id}>
        {label}
        {!required && <span className="el-field__optional"> (optional)</span>}
      </label>

      {hint !== undefined && (
        <p className="el-field__hint" id={hintId}>
          {hint}
        </p>
      )}

      <input
        id={id}
        type={type}
        className="el-field__control"
        aria-invalid={error === undefined ? undefined : true}
        aria-describedby={describedBy === '' ? undefined : describedBy}
        required={required}
        {...props}
      />

      {/*
        The live region exists whether or not there is an error, so that inserting
        a message into an already-rendered region is announced. Creating the region
        and its content at the same time is unreliable across screen readers.
      */}
      <div id={errorId} role="alert" aria-live="polite">
        {error !== undefined && (
          <p className="el-field__error">
            <IconError size={14} />
            <span>{error}</span>
          </p>
        )}
      </div>
    </div>
  );
}
