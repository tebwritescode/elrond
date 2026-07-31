import { inflateSync } from 'node:zlib';

/**
 * Builds a structurally valid PDF: catalog, page tree, pages, content streams,
 * and a correct xref table.
 *
 * A `%PDF-1.7` header followed by `%%EOF` looks like a PDF to a magic-byte
 * sniffer and is rejected by every real parser, so a fixture built that way
 * proves nothing about whether a binder can be assembled from it. These fixtures
 * are parseable, which is what makes the merge step a real test.
 */
export function makePdf(title: string, pageCount: number): Buffer {
  const objects: (string | null)[] = [null, null]; // 1 = catalog, 2 = page tree
  const add = (body: string): number => objects.push(body);

  const font = add('<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>');

  const pageIds: number[] = [];
  for (let page = 1; page <= pageCount; page += 1) {
    const text = `${title} - page ${String(page)} of ${String(pageCount)}`;
    const stream = `BT /F1 24 Tf 72 720 Td (${escapeLiteral(text)}) Tj ET`;
    const contents = add(
      `<< /Length ${String(Buffer.byteLength(stream))} >>\nstream\n${stream}\nendstream`,
    );
    pageIds.push(
      add(
        `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] ` +
          `/Resources << /Font << /F1 ${String(font)} 0 R >> >> ` +
          `/Contents ${String(contents)} 0 R >>`,
      ),
    );
  }

  objects[0] = '<< /Type /Catalog /Pages 2 0 R >>';
  objects[1] =
    `<< /Type /Pages /Kids [${pageIds.map((id) => `${String(id)} 0 R`).join(' ')}] ` +
    `/Count ${String(pageCount)} >>`;

  let pdf = Buffer.from('%PDF-1.7\n%\xe2\xe3\xcf\xd3\n', 'binary');
  const offsets: number[] = [];
  objects.forEach((body, index) => {
    offsets.push(pdf.length);
    pdf = Buffer.concat([
      pdf,
      Buffer.from(`${String(index + 1)} 0 obj\n${body ?? ''}\nendobj\n`, 'binary'),
    ]);
  });

  const xrefStart = pdf.length;
  const size = objects.length + 1;
  let xref = `xref\n0 ${String(size)}\n0000000000 65535 f \n`;
  for (const offset of offsets) {
    xref += `${String(offset).padStart(10, '0')} 00000 n \n`;
  }
  xref +=
    `trailer\n<< /Size ${String(size)} /Root 1 0 R >>\n` +
    `startxref\n${String(xrefStart)}\n%%EOF\n`;

  return Buffer.concat([pdf, Buffer.from(xref, 'binary')]);
}

/**
 * Returns the literal strings drawn by each content stream in a PDF, in file
 * order — enough to assert which pages a generated binder actually contains
 * without depending on a PDF rendering library.
 */
export function drawnText(pdf: Buffer): string[][] {
  const source = pdf.toString('binary');
  const streams: string[][] = [];
  const boundary = /stream\r?\n/g;
  let match: RegExpExecArray | null;

  while ((match = boundary.exec(source)) !== null) {
    const start = match.index + match[0].length;
    const end = source.indexOf('endstream', start);
    if (end < 0) {
      continue;
    }

    let data = Buffer.from(source.slice(start, end), 'binary');
    try {
      data = inflateSync(data);
    } catch {
      // Not deflated; the raw bytes are already the content stream.
    }

    const shown = [...data.toString('binary').matchAll(/\(((?:[^()\\]|\\.)*)\)\s*Tj/g)].map(
      (m) => unescapeLiteral(m[1] ?? ''),
    );
    if (shown.length > 0) {
      streams.push(shown);
    }
  }

  return streams;
}

/** Counts pages by way of the page tree's `/Count`. */
export function pageCount(pdf: Buffer): number {
  const counts = [
    ...pdf.toString('binary').matchAll(/\/Type\s*\/Pages\b[^>]*\/Count\s+(\d+)/g),
  ].map((m) => Number(m[1]));
  return counts.length === 0 ? 0 : Math.max(...counts);
}

function escapeLiteral(text: string): string {
  return text.replace(/[()\\]/g, (char) => `\\${char}`);
}

function unescapeLiteral(text: string): string {
  return text.replace(/\\([()\\])/g, '$1');
}
