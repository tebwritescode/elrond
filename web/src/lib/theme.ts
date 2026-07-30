/** Theme preference storage and application. */

/** What the user asked for, as opposed to what is currently rendered. */
export type ThemePreference = 'system' | 'light' | 'dark';

/** The two themes that can actually be rendered. */
export type ResolvedTheme = 'light' | 'dark';

/**
 * Storage key.
 *
 * `localStorage` is scoped per origin including the port, so this cannot collide
 * with another Elrond build on the same host, but the key is namespaced anyway.
 */
const STORAGE_KEY = 'elrond-alt.theme';

/** Whether a value is a valid preference. */
function isPreference(value: unknown): value is ThemePreference {
  return value === 'system' || value === 'light' || value === 'dark';
}

/** Reads the stored preference, defaulting to following the system. */
export function readStoredPreference(): ThemePreference {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    return isPreference(stored) ? stored : 'system';
  } catch {
    // Private browsing modes can throw on access. Following the system is a
    // perfectly good fallback, so this is not worth surfacing.
    return 'system';
  }
}

/** Persists the preference, ignoring storage failures. */
export function storePreference(preference: ThemePreference): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, preference);
  } catch {
    // Preference is still applied for this session; only persistence is lost.
  }
}

/** Reads the operating system's current preference. */
export function readSystemTheme(): ResolvedTheme {
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

/**
 * Applies a preference to the document.
 *
 * A `system` preference removes the attribute entirely rather than writing the
 * resolved value, so the CSS media query stays in charge and the page follows a
 * later change to the OS setting without any JavaScript running.
 */
export function applyPreference(preference: ThemePreference): void {
  const root = document.documentElement;
  if (preference === 'system') {
    root.removeAttribute('data-theme');
  } else {
    root.setAttribute('data-theme', preference);
  }
}

/** Resolves a preference to the theme that will actually render. */
export function resolveTheme(preference: ThemePreference): ResolvedTheme {
  return preference === 'system' ? readSystemTheme() : preference;
}
