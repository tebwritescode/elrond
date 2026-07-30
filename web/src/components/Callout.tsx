import type { ReactNode } from 'react';

import { IconError, IconInfo, IconSuccess, IconWarning } from './Icon';

export type CalloutTone = 'info' | 'success' | 'caution' | 'danger';

export interface CalloutProps {
  readonly tone?: CalloutTone;
  /** Short heading. Also names the tone in words, so colour is never the only cue. */
  readonly title?: string;
  readonly children: ReactNode;
  /**
   * Whether to interrupt a screen reader.
   *
   * `assertive` is reserved for a failure the user must deal with before
   * continuing; everything else waits its turn.
   */
  readonly urgency?: 'polite' | 'assertive';
}

const ICONS = {
  info: IconInfo,
  success: IconSuccess,
  caution: IconWarning,
  danger: IconError,
} as const;

/** A bordered message block. */
export function Callout({ tone = 'info', title, children, urgency }: CalloutProps) {
  const Icon = ICONS[tone];
  const resolvedUrgency = urgency ?? (tone === 'danger' ? 'assertive' : 'polite');

  return (
    <div
      className={`el-callout el-callout--${tone}`}
      role={resolvedUrgency === 'assertive' ? 'alert' : 'status'}
    >
      <span className="el-callout__icon">
        <Icon size={18} />
      </span>
      <div className="el-callout__body">
        {title !== undefined && <p className="el-callout__title">{title}</p>}
        <div>{children}</div>
      </div>
    </div>
  );
}
