import { expect, test } from '@playwright/test';

/**
 * First sign-in against a server whose administrator came from
 * `ELROND_ADMIN_USERNAME` / `ELROND_ADMIN_PASSWORD` rather than the setup
 * screen.
 *
 * Runs only when `ELROND_E2E_SEEDED` names the expected username, because it
 * needs a server started with those variables — the opposite starting state
 * from the main suite, which requires a library with no accounts at all:
 *
 *     docker run -d -p 3101:3100 \
 *       -e ELROND_PUBLIC_URL=http://127.0.0.1:3101 \
 *       -e ELROND_ADMIN_USERNAME=records.admin \
 *       -e ELROND_ADMIN_PASSWORD='a sufficiently long passphrase' \
 *       tebwritescode/elrond:alt-beta
 *     ELROND_E2E_URL=http://127.0.0.1:3101 ELROND_E2E_SEEDED=records.admin \
 *       npx playwright test seeded-admin
 */

const USERNAME = process.env.ELROND_E2E_SEEDED;
const PASSWORD = process.env.ELROND_E2E_SEEDED_PASSWORD ?? 'a sufficiently long passphrase';

test.skip(
  USERNAME === undefined,
  'needs a server seeded via ELROND_ADMIN_USERNAME; set ELROND_E2E_SEEDED to run',
);

test('a seeded administrator signs straight in', async ({ page }) => {
  await page.goto('/');

  // Setup is already complete, so the first screen is sign-in — the setup
  // screen appearing here would mean the seed did not run.
  await expect(page.getByRole('heading', { name: 'Sign in' })).toBeVisible();

  await page.getByLabel('Username').fill(USERNAME ?? '');
  await page.getByLabel('Password', { exact: true }).fill(PASSWORD);
  await page.getByRole('button', { name: 'Sign in' }).click();

  // A working session, with full administrator navigation.
  await expect(page.getByRole('navigation', { name: 'Primary' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Accounts' })).toBeVisible();
});
