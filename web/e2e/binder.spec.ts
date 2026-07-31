import { readFile } from 'node:fs/promises';

import { expect, test, type Page } from '@playwright/test';

import { drawnText, makePdf, pageCount } from './pdf';

/**
 * The whole promised workflow, in a real browser, against a running server:
 * first-run setup, creating categories, uploading PDFs into them, and printing a
 * combined binder with full-page category separators.
 *
 * This is deliberately one long test rather than several short ones. Every step
 * depends on the state the previous step left behind, and a library can only be
 * set up once, so splitting it would either mean re-running setup (impossible)
 * or hidden ordering dependencies between tests (worse than an explicit
 * sequence).
 */

const PASSWORD = 'a sufficiently long passphrase';

const DOCUMENTS = [
  { category: 'Policies', title: 'Access Policy', pages: 2 },
  { category: 'Policies', title: 'Retention Policy', pages: 3 },
  { category: 'Board Minutes', title: 'January Minutes', pages: 1 },
] as const;

test('upload, categorise, and print a combined binder', async ({ page }) => {
  await test.step('first-run setup creates the administrator', async () => {
    await page.goto('/');

    // A fresh library redirects to setup; one that already has an account shows
    // the sign-in form, which means the server was not started clean.
    await expect(
      page.getByRole('heading', { name: 'Set up your library' }),
      'the server must be a fresh instance with no accounts',
    ).toBeVisible();

    await page.getByLabel('Username').fill('records.admin');
    await page.getByLabel('Password', { exact: true }).fill(PASSWORD);
    await page.getByLabel('Confirm password').fill(PASSWORD);
    await page.getByRole('button', { name: /create|set up|continue/i }).click();

    await expect(page.getByRole('navigation', { name: 'Primary' })).toBeVisible();
  });

  await test.step('create the categories', async () => {
    await page.getByRole('link', { name: 'Documents' }).click();
    await expect(page.getByRole('navigation', { name: 'Categories' })).toBeVisible();

    for (const name of ['Policies', 'Board Minutes']) {
      // Always create at the top level: selecting "All documents" first clears
      // any category the previous iteration left selected, which would otherwise
      // nest the second category inside the first.
      await page.getByRole('button', { name: startingWith('All documents') }).click();
      await page.getByRole('button', { name: 'New category' }).click();
      await page.getByLabel(/^New category/).fill(name);
      await page.getByRole('button', { name: 'Create', exact: true }).click();

      await expect(
        page.getByRole('navigation', { name: 'Categories' }).getByRole('button', {
          name: startingWith(name),
        }),
      ).toBeVisible();
    }
  });

  await test.step('upload a PDF into each category', async () => {
    for (const document of DOCUMENTS) {
      await selectCategory(page, document.category);

      await page.getByRole('button', { name: 'Upload document' }).click();
      await page.getByLabel(/^File/).setInputFiles({
        name: `${document.title.toLowerCase().replace(/ /g, '-')}.pdf`,
        mimeType: 'application/pdf',
        buffer: makePdf(document.title, document.pages),
      });
      await page.getByLabel(/^Title/).fill(document.title);
      await page
        .getByRole('button', { name: /upload/i })
        .last()
        .click();

      await expect(page.getByText(`Uploaded ${document.title}`)).toBeVisible();
      await page.getByRole('button', { name: 'Close upload' }).click();
    }

    // Filed where they were put, not merely uploaded. The title is the row's
    // header cell, and its accessible name carries the filename after the title.
    await selectCategory(page, 'Policies');
    await expect(
      page.getByRole('rowheader', { name: startingWith('Access Policy') }),
    ).toBeVisible();
    await expect(
      page.getByRole('rowheader', { name: startingWith('Retention Policy') }),
    ).toBeVisible();
    await expect(
      page.getByRole('rowheader', { name: startingWith('January Minutes') }),
    ).toHaveCount(0);
  });

  const pdf = await test.step('build and download the binder', async () => {
    await page.getByRole('link', { name: 'Binders' }).click();

    for (const name of ['Policies', 'Board Minutes']) {
      await page.getByRole('checkbox', { name: startingWith(name) }).check();
    }
    await page.getByLabel(/^Title/).fill('Records Binder 2026');
    await page.getByLabel(/^Subtitle/).fill('Verification build');

    for (const toggle of [
      'Front cover',
      'Table of contents',
      'Full-page category separators',
    ]) {
      await expect(page.getByRole('checkbox', { name: toggle })).toBeChecked();
    }

    const download = page.waitForEvent('download');
    await page.getByRole('button', { name: /build|print|download/i }).click();
    const file = await (await download).path();

    return readFile(file);
  });

  await test.step('the binder is a valid PDF with the expected structure', () => {
    expect(pdf.subarray(0, 5).toString('latin1')).toBe('%PDF-');
    expect(pdf.toString('latin1')).toContain('%%EOF');

    // 1 cover + 1 contents + 2 separators + (2 + 3 + 1) document pages.
    expect(pageCount(pdf)).toBe(10);

    const streams = drawnText(pdf);
    const flat = streams.flat();

    expect(flat).toContain('Records Binder 2026');
    expect(flat).toContain('Verification build');

    // Every uploaded page survived the merge, in order, with its own text.
    for (const document of DOCUMENTS) {
      for (let n = 1; n <= document.pages; n += 1) {
        expect(flat).toContain(
          `${document.title} - page ${String(n)} of ${String(document.pages)}`,
        );
      }
    }

    // Each category separator is a full page: its own content stream carrying
    // the category name and nothing else. This is the "full page category
    // separation" requirement, and it is why the check is per-stream rather than
    // a substring search over the whole file.
    for (const name of ['Policies', 'Board Minutes']) {
      expect(
        streams.some((stream) => stream.length === 1 && stream[0] === name),
        `expected a full-page separator for ${name}`,
      ).toBe(true);
    }

    // The contents page cross-references have to match where things actually
    // landed, or the binder is unusable in print.
    const contents = streams.find((stream) => stream[0] === 'Contents');
    expect(contents, 'the binder should have a contents page').toBeDefined();
    expect(contents).toEqual([
      'Contents',
      'Policies',
      '3',
      'Access Policy',
      '4',
      'Retention Policy',
      '6',
      'Board Minutes',
      '9',
      'January Minutes',
      '10',
    ]);

    // Page numbers stamped over the merged content, one per page.
    for (let n = 1; n <= 10; n += 1) {
      expect(streams.some((stream) => stream.length === 1 && stream[0] === String(n))).toBe(
        true,
      );
    }
  });
});

/** Selects a category in the sidebar tree and waits for the filter to apply. */
async function selectCategory(page: Page, name: string): Promise<void> {
  const tree = page.getByRole('navigation', { name: 'Categories' });
  const item = tree.getByRole('button', { name: startingWith(name) });
  await item.click();
  await expect(item).toHaveAttribute('aria-current', 'true');
}

/**
 * Matches an accessible name that begins with `text`.
 *
 * Tree items and binder checkboxes append a rolled-up document count to their
 * label, so their accessible name is "Policies 2" rather than "Policies". An
 * exact match would never fire, and a bare substring match would also catch the
 * "Expand Policies" disclosure button on a category that has children.
 */
function startingWith(text: string): RegExp {
  return new RegExp(`^${text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\b`);
}
