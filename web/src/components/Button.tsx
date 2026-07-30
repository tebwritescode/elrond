import type { ButtonHTMLAttributes, ReactNode } from 'react';

import { Spinner } from './Spinner';

export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';
export type ButtonSize = 'sm' | 'md' | 'lg';

export interface ButtonProps extends Omit<
  ButtonHTMLAttributes<HTMLButtonElement>,
  'className'
> {
  readonly variant?: ButtonVariant;
  readonly size?: ButtonSize;
  /** Stretches the button to its container's width. */
  readonly block?: boolean;
  /**
   * Shows a busy indicator and blocks activation.
   *
   * The button stays in the accessibility tree as a disabled control with
   * `aria-busy`, rather than being replaced by a spinner, so focus is not lost
   * mid-submission.
   */
  readonly isLoading?: boolean;
  /** Label shown while loading. Defaults to the normal label. */
  readonly loadingLabel?: string;
  readonly icon?: ReactNode;
  readonly children: ReactNode;
}

/** The one button in the system. Every action surface uses it. */
export function Button({
  variant = 'secondary',
  size = 'md',
  block = false,
  isLoading = false,
  loadingLabel,
  icon,
  children,
  disabled,
  type = 'button',
  ...props
}: ButtonProps) {
  const classes = [
    'el-button',
    `el-button--${variant}`,
    size === 'md' ? '' : `el-button--${size}`,
    block ? 'el-button--block' : '',
  ]
    .filter(Boolean)
    .join(' ');

  return (
    <button
      type={type}
      className={classes}
      disabled={disabled === true || isLoading}
      aria-busy={isLoading || undefined}
      {...props}
    >
      {isLoading ? <Spinner /> : icon}
      <span>{isLoading ? (loadingLabel ?? children) : children}</span>
    </button>
  );
}
