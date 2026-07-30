import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';

import {
  applyPreference,
  readStoredPreference,
  readSystemTheme,
  resolveTheme,
  storePreference,
  type ResolvedTheme,
  type ThemePreference,
} from '@/lib/theme';

import { ThemeContext, type ThemeContextValue } from './theme-context';

/** Supplies theme state and keeps the document attribute in step with it. */
export function ThemeProvider({ children }: { children: ReactNode }) {
  const [preference, setPreferenceState] = useState<ThemePreference>(readStoredPreference);
  const [systemTheme, setSystemTheme] = useState<ResolvedTheme>(readSystemTheme);

  useEffect(() => {
    applyPreference(preference);
  }, [preference]);

  // Track the OS setting so the toggle can show what "system" currently means.
  useEffect(() => {
    const query = window.matchMedia('(prefers-color-scheme: dark)');
    const onChange = (event: MediaQueryListEvent) => {
      setSystemTheme(event.matches ? 'dark' : 'light');
    };
    query.addEventListener('change', onChange);
    return () => {
      query.removeEventListener('change', onChange);
    };
  }, []);

  const setPreference = useCallback((next: ThemePreference) => {
    setPreferenceState(next);
    storePreference(next);
  }, []);

  const value = useMemo<ThemeContextValue>(
    () => ({
      preference,
      resolved: preference === 'system' ? systemTheme : resolveTheme(preference),
      setPreference,
    }),
    [preference, systemTheme, setPreference],
  );

  return <ThemeContext value={value}>{children}</ThemeContext>;
}
