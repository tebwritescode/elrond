import { readFile } from 'node:fs/promises';

import { expect, test, type Page } from '@playwright/test';

import { drawnText, makePdf, pageCount } from './pdf';
import { makeZip } from './zip';

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

  const fixtures = new Map<string, Buffer>();

  await test.step('upload a PDF into each category', async () => {
    for (const document of DOCUMENTS) {
      fixtures.set(document.title, makePdf(document.title, document.pages));
      await selectCategory(page, document.category);

      await page.getByRole('button', { name: 'Upload document' }).click();
      await page.getByLabel(/^File/).setInputFiles({
        name: `${document.title.toLowerCase().replace(/ /g, '-')}.pdf`,
        mimeType: 'application/pdf',
        buffer: fixtures.get(document.title) ?? Buffer.alloc(0),
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

  await test.step('the stored original downloads byte-for-byte', async () => {
    // Immutability of originals is the project's core promise. Fetch the
    // Download link through the browser's own session and compare against the
    // exact bytes that were uploaded — visibility of a link proves nothing
    // about what it serves.
    const href = await page
      .getByRole('row', { name: startingWith('Access Policy') })
      .getByRole('link', { name: 'Download' })
      .getAttribute('href');
    if (href === null) {
      throw new Error('the document row has no download link');
    }

    const response = await page.request.get(href);
    expect(response.status()).toBe(200);
    expect(response.headers()['content-type']).toBe('application/pdf');

    const uploaded = fixtures.get('Access Policy');
    if (uploaded === undefined) {
      throw new Error('fixture missing');
    }
    expect(Buffer.compare(await response.body(), uploaded)).toBe(0);
  });

  await test.step('a ZIP of folders imports as categories and documents', async () => {
    // Import at the top level, so the archive's own folders shape the tree.
    await page.getByRole('button', { name: startingWith('All documents') }).click();
    await page.getByRole('button', { name: 'Upload document' }).click();
    await page.getByLabel(/^File/).setInputFiles({
      name: 'quarterly.zip',
      mimeType: 'application/zip',
      buffer: makeZip([
        ['Imported/Q1/Quarterly Report.pdf', makePdf('Quarterly Report', 1)],
        ['Imported/junk.exe', Buffer.from('MZ, not a document')],
      ]),
    });
    await page.getByRole('button', { name: 'Import archive' }).click();

    // The archive imported, and the junk inside it was reported, not fatal.
    await expect(page.getByText('Imported 1 document from the archive')).toBeVisible();
    await expect(page.getByText('Imported/junk.exe')).toBeVisible();

    // The folder chain arrived as nested categories.
    const tree = page.getByRole('navigation', { name: 'Categories' });
    await expect(tree.getByRole('button', { name: startingWith('Imported') })).toBeVisible();
    await expect(tree.getByRole('button', { name: startingWith('Q1') })).toBeVisible();
    await page.getByRole('button', { name: 'Close upload' }).click();
  });

  await test.step('clicking a document title opens the PDF itself', async () => {
    await selectCategory(page, 'Q1');

    const href = await page
      .getByRole('link', { name: 'Quarterly Report' })
      .getAttribute('href');
    if (href === null) {
      throw new Error('the document title is not a link');
    }

    // Served inline as a PDF: this is what the browser renders in the new tab.
    const response = await page.request.get(href);
    expect(response.status()).toBe(200);
    expect(response.headers()['content-type']).toBe('application/pdf');
    expect(response.headers()['content-disposition'] ?? '').toContain('inline');
    expect((await response.body()).subarray(0, 5).toString('latin1')).toBe('%PDF-');
  });

  await test.step('the info button shows details without opening the document', async () => {
    await page.getByRole('button', { name: 'Details for Quarterly Report' }).click();

    // Provenance from the archive, and the explicit open action.
    await expect(page.getByText('Imported/Q1/Quarterly Report.pdf')).toBeVisible();
    await expect(page.getByRole('link', { name: 'Open document' })).toBeVisible();
    await expect(page.getByText('1 version')).toBeVisible();

    await page.getByRole('button', { name: 'Close', exact: true }).click();
    await expect(page.getByRole('link', { name: 'Open document' })).toHaveCount(0);
  });

  await test.step('categories can be renamed and deleted from the sidebar', async () => {
    const tree = page.getByRole('navigation', { name: 'Categories' });

    // Rename the imported root.
    await selectCategory(page, 'Imported');
    await page.getByRole('button', { name: 'Rename category' }).click();
    await page.getByLabel(/^Rename/).fill('Archive 2026');
    await page.getByRole('button', { name: 'Rename', exact: true }).click();
    await expect(
      tree.getByRole('button', { name: startingWith('Archive 2026') }),
    ).toBeVisible();

    // A scratch category can be deleted again while it is empty.
    await page.getByRole('button', { name: startingWith('All documents') }).click();
    await page.getByRole('button', { name: 'New category' }).click();
    await page.getByLabel(/^New category/).fill('Scratch');
    await page.getByRole('button', { name: 'Create', exact: true }).click();
    await selectCategory(page, 'Scratch');
    await page.getByRole('button', { name: 'Delete category' }).click();
    await expect(tree.getByRole('button', { name: startingWith('Scratch') })).toHaveCount(0);
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
      'Full-page document separators',
    ]) {
      await expect(page.getByRole('checkbox', { name: startingWith(toggle) })).toBeChecked();
    }

    const download = page.waitForEvent('download');
    await page.getByRole('button', { name: /build|print|download/i }).click();
    const file = await (await download).path();

    return readFile(file);
  });

  await test.step('the binder is a valid PDF with the expected structure', () => {
    expect(pdf.subarray(0, 5).toString('latin1')).toBe('%PDF-');
    expect(pdf.toString('latin1')).toContain('%%EOF');

    // 1 cover + 1 contents + 2 category separators + 3 document separators
    // + (2 + 3 + 1) document pages.
    expect(pageCount(pdf)).toBe(13);

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
    // the category name and nothing else. The title is set as large as fits, so
    // it may wrap across lines — the strings joined back together must be the
    // name, with nothing extra on the page. This is the "full page category
    // separation" requirement, and it is why the check is per-stream rather
    // than a substring search over the whole file.
    for (const name of ['Policies', 'Board Minutes']) {
      expect(
        streams.some((stream) => stream.join(' ') === name),
        `expected a full-page separator for ${name}`,
      ).toBe(true);
    }

    // Each document gets a full-page separator of its own: the category path
    // above, then the title, and nothing else.
    for (const document of DOCUMENTS) {
      expect(
        streams.some(
          (stream) =>
            stream[0] === document.category && stream.slice(1).join(' ') === document.title,
        ),
        `expected a full-page separator for ${document.title}`,
      ).toBe(true);
    }

    // The contents page cross-references have to match where things actually
    // landed, or the binder is unusable in print. Each document's entry points
    // at its separator page — the page a reader turns to.
    const contents = streams.find((stream) => stream[0] === 'Contents');
    expect(contents, 'the binder should have a contents page').toBeDefined();
    expect(contents).toEqual([
      'Contents',
      'Policies',
      '3',
      'Access Policy',
      '4',
      'Retention Policy',
      '7',
      'Board Minutes',
      '11',
      'January Minutes',
      '12',
    ]);

    // Page numbers stamped over the merged content, one per page.
    for (let n = 1; n <= 13; n += 1) {
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
