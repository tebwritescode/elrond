import { useEffect, useRef, useState, type DragEvent, type FormEvent } from "react";
import { Archive, CheckCircle2, FileArchive, FolderTree, Upload, X } from "lucide-react";
import { importZipArchive, type ImportSummary } from "../../lib/api";

type ZipImportDialogProps = {
  open: boolean;
  onClose: () => void;
  onImported: () => void;
};

export function ZipImportDialog({ open, onClose, onImported }: ZipImportDialogProps) {
  const input = useRef<HTMLInputElement>(null);
  const [archive, setArchive] = useState<File>();
  const [rootCategory, setRootCategory] = useState("Imported");
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

  function selectFile(file?: File) {
    setError(undefined);
    setSummary(undefined);
    if (!file) return;
    if (!file.name.toLowerCase().endsWith(".zip")) {
      setArchive(undefined);
      setError("Choose a file ending in .zip.");
      return;
    }
    if (file.size > 256 * 1024 * 1024) {
      setArchive(undefined);
      setError("The ZIP archive exceeds the 256 MiB upload limit.");
      return;
    }
    setArchive(file);
  }

  function drop(event: DragEvent<HTMLButtonElement>) {
    event.preventDefault();
    setDragging(false);
    selectFile(event.dataTransfer.files[0]);
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!archive) return;
    setSubmitting(true);
    setError(undefined);
    try {
      const result = await importZipArchive(archive, rootCategory);
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
    setArchive(undefined);
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
            <p className="eyebrow">Bulk ingestion</p>
            <h2 id="import-title">Import a folder hierarchy</h2>
          </div>
          <button aria-label="Close import" disabled={submitting} onClick={close} type="button">
            <X size={19} />
          </button>
        </header>

        {summary ? (
          <div className="import-complete">
            <CheckCircle2 size={38} />
            <h3>Import complete</h3>
            <p>The archive was preserved and its folder structure is now part of your library.</p>
            <dl>
              <div><dt>Documents</dt><dd>{summary.documentsImported}</dd></div>
              <div><dt>Categories</dt><dd>{summary.categoriesCreated}</dd></div>
              <div><dt>Duplicates</dt><dd>{summary.duplicatesSkipped}</dd></div>
              <div><dt>Unsupported</dt><dd>{summary.unsupportedSkipped}</dd></div>
            </dl>
            <button className="dialog-submit" onClick={close} type="button">Return to library</button>
          </div>
        ) : (
          <form onSubmit={submit}>
            <div className="hierarchy-explainer">
              <FolderTree size={19} />
              <p><strong>Folders become categories.</strong> Every nested folder becomes a child category, while files keep their containing folder as their primary location.</p>
            </div>

            <input
              accept=".zip,application/zip"
              className="sr-only"
              onChange={(event) => selectFile(event.target.files?.[0])}
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
              <span>{archive ? <FileArchive size={27} /> : <Upload size={27} />}</span>
              {archive ? (
                <><strong>{archive.name}</strong><small>{formatBytes(archive.size)} selected</small></>
              ) : (
                <><strong>Drop a ZIP archive here</strong><small>or choose a file, up to 256 MiB</small></>
              )}
            </button>

            <label className="form-field import-root-field">
              <span>Category for files at the ZIP root</span>
              <input
                maxLength={120}
                onChange={(event) => setRootCategory(event.target.value)}
                required
                value={rootCategory}
              />
              <small>Folders already inside the ZIP retain their own names.</small>
            </label>

            {error && <p className="form-error" role="alert">{error}</p>}

            <footer className="dialog-actions">
              <button className="dialog-cancel" disabled={submitting} onClick={close} type="button">Cancel</button>
              <button className="dialog-submit" disabled={!archive || submitting} type="submit">
                {submitting ? "Validating and importing..." : "Import hierarchy"}
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
