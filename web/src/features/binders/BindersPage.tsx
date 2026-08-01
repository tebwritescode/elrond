import { useState } from "react";
import { BookOpen, Download } from "lucide-react";
import { downloadPrintableBinder } from "../../lib/api";

export function BindersPage() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  const generate = async () => {
    setBusy(true);
    setError(undefined);
    try {
      await downloadPrintableBinder();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "The printable binder could not be generated.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="binder-page">
      <header className="catalog-heading">
        <div>
          <p className="eyebrow">Print production</p>
          <h1>Binder studio</h1>
          <p>Generate one indexed binder from every PDF-ready document in the library.</p>
        </div>
      </header>
      <section className="binder-sheet">
        <div className="binder-mark"><BookOpen size={38} strokeWidth={1.3} /></div>
        <div className="binder-copy">
          <p className="eyebrow">Library binder</p>
          <h2>Categories, separators, and complete documents</h2>
          <p>The download starts with an index, includes category and document separator pages, and includes every page of each latest PDF in category and title order.</p>
          <p className="binder-note">The server verifies and includes every latest PDF-ready document. Documents still awaiting conversion are left out.</p>
          {error && <p className="form-error" role="alert">{error}</p>}
          <button className="primary-button" disabled={busy} onClick={generate} type="button">
            <Download size={17} /> {busy ? "Building binder..." : "Generate printable binder"}
          </button>
        </div>
      </section>
    </div>
  );
}
