import { fileURLToPath, URL } from 'node:url';

import react from '@vitejs/plugin-react';
// `vitest/config` rather than `vite`, because the `test` block below is a Vitest
// extension that Vite's own config type does not know about.
import { defineConfig } from 'vitest/config';

/**
 * Port this build's dev server owns.
 *
 * Deliberately not Vite's default 5173: another Elrond implementation may be
 * running on this host, and `strictPort` makes a collision fail loudly instead
 * of silently landing on a neighbouring port.
 */
const DEV_PORT = 5273;

/** Port the Rust API binds in development. */
const API_PORT = 3100;

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  server: {
    port: DEV_PORT,
    strictPort: true,
    proxy: {
      // Proxying rather than enabling CORS keeps the browser on one origin, so
      // cookies behave in development exactly as they do in the single-process
      // production deployment.
      //
      // The API is restarted constantly by cargo-watch. Vite logs proxy errors
      // itself, and the client's bootstrap query polls while it is failing, so a
      // rebuild shows up as a brief "waiting for the server" banner rather than
      // as a broken page.
      '/api': {
        target: `http://127.0.0.1:${String(API_PORT)}`,
        changeOrigin: false,
      },
    },
  },
  build: {
    outDir: 'dist',
    // Vite writes content-hashed filenames here; the Rust server serves this
    // directory with a one-year immutable cache.
    assetsDir: 'assets',
    sourcemap: true,
    target: 'es2022',
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
    css: true,
    // Unit tests only. `e2e/` is Playwright's, and its specs import a runner
    // vitest cannot provide, so collecting them here fails the run.
    include: ['src/**/*.{test,spec}.{ts,tsx}'],
  },
});
