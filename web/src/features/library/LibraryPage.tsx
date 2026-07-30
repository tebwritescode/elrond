import { useDeferredValue, useState } from "react";
import { ChevronRight, CircleX, File, FileCheck2, FileClock, FileText, LoaderCircle, Search, X } from "lucide-react";
import type { DocumentSummary } from "../../lib/api";

type LibraryPageProps = {
  documents: DocumentSummary[];
  loading: boolean;
  query: string;
  onQueryChange: (query: string) => void;
};

export function LibraryPage({ documents, loading, query, onQueryChange }: LibraryPageProps) {
  const [selectedId, setSelectedId] = useState<string>();
  const selected = documents.find((document) => document.id === selectedId);
  const deferredQuery = useDeferredValue(query.trim().toLocaleLowerCase());
  const visibleDocuments = deferredQuery
    ? documents.filter((document) =>
        [document.title, document.originalFilename, document.categoryName ?? "", document.status]
          .join(" ")
          .toLocaleLowerCase()
          .includes(deferredQuery),
      )
    : documents;

  return (
    <div className="catalog-page">
      <header className="catalog-heading">
        <div>
          <p className="eyebrow">Controlled documents</p>
          <h1>Library</h1>
          <p>Browse source files, PDF derivatives, and the current lifecycle state of every document.</p>
        </div>
        <span>{visibleDocuments.length} document{visibleDocuments.length === 1 ? "" : "s"}</span>
      </header>

      <section className="library-panel">
        <div className="library-toolbar">
          <label>
            <Search size={16} />
            <span className="sr-only">Filter documents</span>
            <input
              onChange={(event) => onQueryChange(event.target.value)}
              placeholder="Filter this view..."
              type="search"
              value={query}
            />
          </label>
          <div className="view-summary"><span>Latest versions</span><span>All categories</span></div>
        </div>

        {loading ? (
          <div className="catalog-loading" aria-label="Loading documents">
            {[1, 2, 3, 4].map((row) => <span key={row} />)}
          </div>
        ) : visibleDocuments.length === 0 ? (
          <div className="library-empty">
            <FileText size={32} strokeWidth={1.4} />
            <h2>{documents.length === 0 ? "Your library is ready" : "No documents match"}</h2>
            <p>{documents.length === 0 ? "Import a ZIP hierarchy to add documents and categories." : "Try a broader title, filename, category, or status."}</p>
          </div>
        ) : (
          <div className="document-table-wrap">
            <table className="document-table">
              <thead><tr><th>Document</th><th>Category</th><th>Status</th><th>Version</th><th>PDF</th><th><span className="sr-only">Open</span></th></tr></thead>
              <tbody>
                {visibleDocuments.map((document) => (
                  <tr key={document.id} onClick={() => setSelectedId(document.id)}>
                    <td><span className="file-glyph"><File size={17} /></span><span><strong>{document.title}</strong><small>{document.originalFilename}</small></span></td>
                    <td>{document.categoryName ?? "Unfiled"}</td>
                    <td><Status status={document.status} /></td>
                    <td>v{document.versionNumber}</td>
                    <td><PdfStatus document={document} /></td>
                    <td><ChevronRight size={17} /></td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      {selected && (
        <aside className="document-inspector" aria-label={`${selected.title} details`}>
          <button aria-label="Close document details" onClick={() => setSelectedId(undefined)} type="button"><X size={19} /></button>
          <div className="inspector-document"><FileText size={34} strokeWidth={1.3} /></div>
          <p className="eyebrow">Document details</p>
          <h2>{selected.title}</h2>
          <p className="inspector-filename">{selected.originalFilename}</p>
          <dl>
            <div><dt>Primary category</dt><dd>{selected.categoryName ?? "Unfiled"}</dd></div>
            <div><dt>Lifecycle</dt><dd><Status status={selected.status} /></dd></div>
            <div><dt>Current version</dt><dd>Version {selected.versionNumber}</dd></div>
            <div><dt>Viewing copy</dt><dd><PdfStatus document={selected} /></dd></div>
          </dl>
          {selected.conversionStatus === "failed" && selected.conversionError && <p role="alert">{selected.conversionError}</p>}
          <button className="inspector-primary" disabled={!selected.hasPdf} type="button">{selected.hasPdf ? "Open document" : "PDF not ready"}</button>
        </aside>
      )}
    </div>
  );
}

function PdfStatus({ document }: { document: DocumentSummary }) {
  if (document.conversionStatus === "ready") return <span className="pdf-ready"><FileCheck2 size={15} /> Ready</span>;
  if (document.conversionStatus === "processing") return <span className="pdf-pending"><LoaderCircle size={15} /> Converting</span>;
  if (document.conversionStatus === "failed") return <span className="pdf-pending"><CircleX size={15} /> Failed</span>;
  return <span className="pdf-pending"><FileClock size={15} /> Queued</span>;
}

function Status({ status }: { status: DocumentSummary["status"] }) {
  return <span className={`document-status ${status}`}>{status.replace("_", " ")}</span>;
}
