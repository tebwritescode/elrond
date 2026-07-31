import { defineConfig, devices } from '@playwright/test';

/**
 * End-to-end configuration.
 *
 * These tests drive a real browser against a running Elrond server — by default
 * the container, which is the artefact that actually ships. Point ELROND_E2E_URL
 * at any instance:
 *
 *     docker run -d -p 3101:3100 tebwritescode/elrond:alt-beta
 *     ELROND_E2E_URL=http://127.0.0.1:3101 npm run test:e2e
 *
 * The server is not started here. A `webServer` block would build and boot the
 * Rust binary on every run, which is both slow and a different artefact from the
 * image users install; testing the image directly is the point.
 */
export default defineConfig({
  testDir: './e2e',
  // Each spec drives a full library through first-run setup, so they must not
  // share a server. Run them one at a time against a single instance.
  fullyParallel: false,
  workers: 1,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [['github'], ['list']] : [['list']],
  timeout: 60_000,
  expect: { timeout: 15_000 },
  use: {
    baseURL: process.env.ELROND_E2E_URL ?? 'http://127.0.0.1:3101',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    // The download assertions are the point of the binder test.
    acceptDownloads: true,
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
});
