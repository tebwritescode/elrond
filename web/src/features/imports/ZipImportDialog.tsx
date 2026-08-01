import { useEffect, useRef, useState, type DragEvent, type FormEvent } from "react";
import { Archive, CheckCircle2, FileArchive, FileUp, FolderTree, Upload, X } from "lucide-react";
import {
  importZipArchive,
  uploadDocuments,
  type CategorySummary,
  type ImportSummary,
} from "../../lib/api";

type ZipImportDialogProps = {
  open: boolean;
  categories: CategorySummary[];
  onClose: () => void;
  onImported: () => void;
};

export function ZipImportDialog({ open, categories, onClose, onImported }: ZipImportDialogProps) {
  const input = useRef<HTMLInputElement>(null);
  const [files, setFiles] = useState<File[]>([]);
  const [mode, setMode] = useState<"document" | "hierarchy">("document");
  const [rootCategory, setRootCategory] = useState("Imported");
  const [categoryId, setCategoryId] = useState("");
  const [dragging, setDragging] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string>();
  const [summary, setSummary] = useState<ImportSummary>();

  useEffect(() => {
    if (!open) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !submitting) onClose();
    };
    document.addEventListener("keydown", closeOnEscape);
    return () => document.removeEventListener("keydown", closeOnEscape);
  }, [onClose, open, submitting]);

  if (!open) return null;

  function selectFiles(selected: File[]) {
    setError(undefined);
    setSummary(undefined);
    if (selected.length === 0) return;
    const chosen = mode === "hierarchy" ? selected.slice(0, 1) : selected;
    const file = chosen[0];
    if (!file) return;
    if (mode === "hierarchy" && !file.name.toLowerCase().endsWith(".zip")) {
      setFiles([]);
      setError("Choose a file ending in .zip.");
      return;
    }
    const supported = ["pdf", "docx", "xlsx", "pptx", "odt", "ods", "odp", "txt", "jpg", "jpeg", "png", "tif", "tiff"];
    const extension = file.name.split(".").pop()?.toLowerCase() ?? "";
    if (mode === "document" && !supported.includes(extension)) {
      setFiles([]);
      setError("This file type is not supported yet.");
      return;
    }
    const limit = mode === "hierarchy" ? 256 * 1024 * 1024 : 100 * 1024 * 1024;
    if (file.size > limit) {
      setFiles([]);
      setError(`The selected file exceeds the ${mode === "hierarchy" ? "256" : "100"} MiB upload limit.`);
      return;
    }
    if (mode === "document" && chosen.some((item) => {
      const itemExtension = item.name.split(".").pop()?.toLowerCase() ?? "";
      return item.size > limit || !supported.includes(itemExtension);
    })) {
      setFiles([]);
      setError("Every selected document must be supported and no larger than 100 MiB.");
      return;
    }
    setFiles(chosen);
  }

  function drop(event: DragEvent<HTMLButtonElement>) {
    event.preventDefault();
    setDragging(false);
    selectFiles(Array.from(event.dataTransfer.files));
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (files.length === 0) return;
    setSubmitting(true);
    setError(undefined);
    try {
      const result = mode === "hierarchy"
        ? await importZipArchive(files[0]!, rootCategory)
        : await uploadDocuments(files, buildCategoryPath(categories, categoryId));
      setSummary(result);
      onImported();
    } catch (caughtError) {
      setError(caughtError instanceof Error ? caughtError.message : "Import failed.");
    } finally {
      setSubmitting(false);
    }
  }

  function close() {
    if (submitting) return;
    setFiles([]);
    setSummary(undefined);
    setError(undefined);
    onClose();
  }

  return (
    <div className="dialog-backdrop" onMouseDown={close} role="presentation">
      <section
        aria-labelledby="import-title"
        aria-modal="true"
        className="import-dialog"
        onMouseDown={(event) => event.stopPropagation()}
        role="dialog"
      >
        <header className="dialog-header">
          <div className="dialog-symbol"><Archive size={22} /></div>
          <div>
            <p className="eyebrow">Document ingestion</p>
            <h2 id="import-title">Import documents</h2>
          </div>
          <button aria-label="Close import" disabled={submitting} onClick={close} type="button">
            <X size={19} />
          </button>
        </header>

        {summary ? (
          <div className="import-complete">
            <CheckCircle2 size={38} />
            <h3>Import complete</h3>
            <p>{mode === "hierarchy" ? "Valid documents were imported with their folder structure; all skips are reported below." : "Valid documents were imported and preserved; all skips are reported below."}</p>
            <dl>
              <div><dt>Documents</dt><dd>{summary.documentsImported}</dd></div>
              <div><dt>Categories</dt><dd>{summary.categoriesCreated}</dd></div>
              <div><dt>Duplicates</dt><dd>{summary.duplicatesSkipped}</dd></div>
              <div><dt>Unsupported</dt><dd>{summary.unsupportedSkipped}</dd></div>
              <div><dt>Invalid files</dt><dd>{summary.invalidSignatureSkipped}</dd></div>
            </dl>
            <button className="dialog-submit" onClick={close} type="button">Return to library</button>
          </div>
        ) : (
          <form onSubmit={submit}>
            <div aria-label="Import type" className="import-tabs" role="tablist">
              <button
                aria-selected={mode === "document"}
                onClick={() => { setMode("document"); setFiles([]); setError(undefined); }}
                role="tab"
                type="button"
              ><FileUp size={16} /> Documents</button>
              <button
                aria-selected={mode === "hierarchy"}
                onClick={() => { setMode("hierarchy"); setFiles([]); setError(undefined); }}
                role="tab"
                type="button"
              ><FolderTree size={16} /> Folder ZIP</button>
            </div>

            {mode === "hierarchy" && (
              <div className="hierarchy-explainer">
                <FolderTree size={19} />
                <p><strong>Folders become categories.</strong> Every nested folder becomes a child category, while files keep their containing folder as their primary location.</p>
              </div>
            )}

            <input
              accept={mode === "hierarchy" ? ".zip,application/zip" : ".pdf,.docx,.xlsx,.pptx,.odt,.ods,.odp,.txt,.jpg,.jpeg,.png,.tif,.tiff"}
              className="sr-only"
              multiple={mode === "document"}
              onChange={(event) => selectFiles(Array.from(event.target.files ?? []))}
              ref={input}
              type="file"
            />
            <button
              className={`archive-dropzone${dragging ? " dragging" : ""}`}
              onClick={() => input.current?.click()}
              onDragEnter={() => setDragging(true)}
              onDragLeave={() => setDragging(false)}
              onDragOver={(event) => event.preventDefault()}
              onDrop={drop}
              type="button"
            >
              <span>{files.length ? (mode === "hierarchy" ? <FileArchive size={27} /> : <FileUp size={27} />) : <Upload size={27} />}</span>
              {files.length ? (
                <><strong>{files.length === 1 ? files[0]!.name : `${files.length} documents selected`}</strong><small>{formatBytes(files.reduce((total, file) => total + file.size, 0))} selected</small></>
              ) : (
                <><strong>{mode === "hierarchy" ? "Drop a ZIP archive here" : "Drop documents here"}</strong><small>or choose files, up to {mode === "hierarchy" ? "256" : "100"} MiB each</small></>
              )}
            </button>

            {mode === "hierarchy" ? <label className="form-field import-root-field">
              <span>Category for files at the ZIP root</span>
              <input
                aria-label="Category for files at the ZIP root"
                maxLength={120}
                onChange={(event) => setRootCategory(event.target.value)}
                required
                value={rootCategory}
              />
              <small>Folders already inside the ZIP retain their own names.</small>
            </label> : <label className="form-field import-root-field">
              <span>Primary category</span>
              <select aria-label="Primary category" onChange={(event) => setCategoryId(event.target.value)} value={categoryId}>
                <option value="">Imported</option>
                {categories.map((category) => <option key={category.id} value={category.id}>{category.name}</option>)}
              </select>
              <small>The original remains unchanged; non-PDF files will receive a PDF viewing copy.</small>
            </label>}

            {error && <p className="form-error" role="alert">{error}</p>}

            <footer className="dialog-actions">
              <button className="dialog-cancel" disabled={submitting} onClick={close} type="button">Cancel</button>
              <button className="dialog-submit" disabled={files.length === 0 || submitting} type="submit">
                {submitting ? "Validating and importing..." : mode === "hierarchy" ? "Import hierarchy" : "Upload documents"}
              </button>
            </footer>
          </form>
        )}
      </section>
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

function buildCategoryPath(categories: CategorySummary[], categoryId: string): string[] {
  if (!categoryId) return ["Imported"];
  const path: string[] = [];
  let current = categories.find((category) => category.id === categoryId);
  while (current) {
    path.unshift(current.name);
    const parentId = current.parentId;
    current = parentId ? categories.find((category) => category.id === parentId) : undefined;
  }
  return path.length > 0 ? path : ["Imported"];
}
