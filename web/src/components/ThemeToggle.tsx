import { useTheme } from '@/app/theme-context';
import type { ThemePreference } from '@/lib/theme';

import { IconMonitor, IconMoon, IconSun } from './Icon';

const OPTIONS: readonly {
  readonly value: ThemePreference;
  readonly label: string;
  readonly Icon: typeof IconSun;
}[] = [
  { value: 'system', label: 'Match system theme', Icon: IconMonitor },
  { value: 'light', label: 'Light theme', Icon: IconSun },
  { value: 'dark', label: 'Dark theme', Icon: IconMoon },
];

/**
 * Three-way theme control.
 *
 * A group of toggle buttons rather than a single cycling button: with one button
 * the current state is ambiguous, and there is no way to get back to "follow the
 * system" once you have left it. Each option is icon-only on screen, so each
 * carries an accessible name.
 */
export function ThemeToggle() {
  const { preference, resolved, setPreference } = useTheme();

  return (
    <div className="el-theme-toggle" role="group" aria-label="Colour theme">
      {OPTIONS.map(({ value, label, Icon }) => (
        <button
          key={value}
          type="button"
          className="el-theme-toggle__option"
          aria-pressed={preference === value}
          aria-label={value === 'system' ? `${label} (currently ${resolved})` : label}
          onClick={() => {
            setPreference(value);
          }}
        >
          <Icon size={15} />
        </button>
      ))}
    </div>
  );
}
