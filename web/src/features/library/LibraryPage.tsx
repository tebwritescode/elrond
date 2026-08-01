import { useDeferredValue, useEffect, useState, type FormEvent } from "react";
import { ChevronRight, CircleX, Download, File, FileCheck2, FileClock, FileText, LoaderCircle, Search, X } from "lucide-react";
import { updateDocument, type CategorySummary, type DocumentSummary } from "../../lib/api";

type LibraryPageProps = {
  documents: DocumentSummary[];
  categories: CategorySummary[];
  loading: boolean;
  query: string;
  onCatalogReload: () => void;
  onQueryChange: (query: string) => void;
};

export function LibraryPage({ categories, documents, loading, onCatalogReload, query, onQueryChange }: LibraryPageProps) {
  const [selectedId, setSelectedId] = useState<string>();
  const selected = documents.find((document) => document.id === selectedId);
  const deferredQuery = useDeferredValue(query.trim().toLocaleLowerCase());
  const visibleDocuments = deferredQuery
    ? documents.filter((document) =>
        [document.title, document.originalFilename, document.categoryName ?? "", document.status, ...document.tags]
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
                    <td><span className="file-glyph"><File size={17} /></span><span><strong>{document.title}</strong><small>{document.originalFilename}</small><TagList tags={document.tags} /></span></td>
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
            <div><dt>Lifecycle</dt><dd><Status status={selected.status} /></dd></div>
            <div><dt>Current version</dt><dd>Version {selected.versionNumber}</dd></div>
            <div><dt>Viewing copy</dt><dd><PdfStatus document={selected} /></dd></div>
          </dl>
          {selected.conversionStatus === "failed" && selected.conversionError && <p role="alert">{selected.conversionError}</p>}
          <DocumentMetadataForm categories={categories} document={selected} onCatalogReload={onCatalogReload} />
          <button className="inspector-primary" disabled={!selected.hasPdf} onClick={() => window.open(`/api/v1/documents/${encodeURIComponent(selected.id)}/pdf`, "_blank", "noopener,noreferrer")} type="button">{selected.hasPdf ? "Open document" : "PDF not ready"}</button>
          <a className="inspector-download" href={`/api/v1/documents/${encodeURIComponent(selected.id)}/original`}><Download size={15} /> Download original</a>
        </aside>
      )}
    </div>
  );
}

function DocumentMetadataForm({ categories, document, onCatalogReload }: { categories: CategorySummary[]; document: DocumentSummary; onCatalogReload: () => void }) {
  const [categoryId, setCategoryId] = useState(document.categoryId ?? "");
  const [tags, setTags] = useState(document.tags.join(", "));
  const [saving, setSaving] = useState(false);
  const [feedback, setFeedback] = useState<{ kind: "success" | "error"; text: string }>();

  useEffect(() => {
    setCategoryId(document.categoryId ?? "");
    setTags(document.tags.join(", "));
  }, [document.categoryId, document.id, document.tags]);

  useEffect(() => {
    setFeedback(undefined);
  }, [document.id]);

  async function save(event: FormEvent) {
    event.preventDefault();
    setSaving(true);
    setFeedback(undefined);
    const normalizedTags = [...new Set(tags.split(",").map((tag) => tag.trim()).filter(Boolean))];
    try {
      await updateDocument(document.id, categoryId || null, normalizedTags);
      setFeedback({ kind: "success", text: "Document details saved." });
      onCatalogReload();
    } catch (error) {
      setFeedback({ kind: "error", text: error instanceof Error ? error.message : "The document could not be updated." });
    } finally {
      setSaving(false);
    }
  }

  return <form className="document-metadata-form" onSubmit={save}>
    <label><span>Primary category</span><select aria-label="Primary category" onChange={(event) => setCategoryId(event.target.value)} value={categoryId}><option value="">Unfiled</option>{categories.map((category) => <option key={category.id} value={category.id}>{categoryPath(categories, category)}</option>)}</select></label>
    <label><span>Tags</span><input aria-label="Tags" onChange={(event) => setTags(event.target.value)} placeholder="policy, safety, annual" value={tags} /><small>Separate tags with commas.</small></label>
    {feedback && <p className={feedback.kind === "error" ? "form-error" : "form-success"} role={feedback.kind === "error" ? "alert" : "status"}>{feedback.text}</p>}
    <button className="secondary-button" disabled={saving} type="submit">{saving ? "Saving..." : "Save details"}</button>
  </form>;
}

function categoryPath(categories: CategorySummary[], category: CategorySummary): string {
  const names = [category.name];
  let parentId = category.parentId;
  const visited = new Set([category.id]);
  while (parentId) {
    const parent = categories.find((candidate) => candidate.id === parentId);
    if (!parent || visited.has(parent.id)) break;
    visited.add(parent.id);
    names.unshift(parent.name);
    parentId = parent.parentId;
  }
  return names.join(" / ");
}

function TagList({ tags }: { tags: string[] }) {
  if (tags.length === 0) return null;
  return <span className="document-tags">{tags.map((tag) => <span key={tag}>{tag}</span>)}</span>;
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
