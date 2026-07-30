import { createContext, use } from 'react';

import type { ResolvedTheme, ThemePreference } from '@/lib/theme';

/** Theme state and the setter for it. */
export interface ThemeContextValue {
  readonly preference: ThemePreference;
  readonly resolved: ResolvedTheme;
  readonly setPreference: (preference: ThemePreference) => void;
}

export const ThemeContext = createContext<ThemeContextValue | null>(null);

/** Reads theme state. Throws if used outside the provider, which is a wiring bug. */
export function useTheme(): ThemeContextValue {
  const value = use(ThemeContext);
  if (value === null) {
    throw new Error('useTheme must be used inside <ThemeProvider>');
  }
  return value;
}
