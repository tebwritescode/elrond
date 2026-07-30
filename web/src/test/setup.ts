import '@testing-library/jest-dom/vitest';

import { afterEach, vi } from 'vitest';

/**
 * jsdom does not implement `matchMedia`, which the theme provider queries on
 * mount. A stub that reports "light" keeps component tests deterministic.
 */
Object.defineProperty(window, 'matchMedia', {
  writable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    addListener: () => undefined,
    removeListener: () => undefined,
    dispatchEvent: () => false,
  }),
});

afterEach(() => {
  vi.restoreAllMocks();
});
