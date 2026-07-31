import { useState } from "react";
import { ArrowRight, BookOpen, FileCheck2, FileText, FolderTree, Plus, Sparkles } from "lucide-react";
import type { LibraryOverview } from "../../lib/api";
import { SetupDialog } from "../setup/SetupDialog";

type LoadState =
  | { status: "loading" }
  | { status: "ready"; overview: LibraryOverview }
  | { status: "offline" };

type DashboardPageProps = {
  loadState: LoadState;
  onImport: () => void;
  onSetupComplete: () => void;
};

const starterActions = [
  {
    icon: FileText,
    title: "Add your first document",
    detail: "Preserve its source and prepare a PDF viewing copy.",
  },
  {
    icon: FolderTree,
    title: "Shape your category tree",
    detail: "Create a hierarchy that reflects how your procedures work.",
  },
  {
    icon: BookOpen,
    title: "Compose a binder",
    detail: "Arrange controlled versions into a publication-ready volume.",
  },
];

export function DashboardPage({ loadState, onImport, onSetupComplete }: DashboardPageProps) {
  const [setupOpen, setSetupOpen] = useState(false);
  const overview = loadState.status === "ready" ? loadState.overview : undefined;

  return (
    <div className="dashboard-page">
      <section className="page-introduction">
        <div>
          <p className="eyebrow">Library overview</p>
          <h1>Your working collection</h1>
          <p className="page-description">
            Preserve source material, control published versions, and assemble documentation that stays current.
          </p>
        </div>
        <div className="date-block" aria-label="Current development release">
          <span>FOUNDATION RELEASE</span>
          <strong>v0.4.6</strong>
        </div>
      </section>

      {overview?.setupRequired && (
        <section className="setup-callout">
          <div className="setup-icon"><Sparkles size={21} /></div>
          <div>
            <p className="eyebrow">First-run setup</p>
            <h2>Make this library yours</h2>
            <p>Create the local administrator and choose the defaults used for document numbering and PDF processing.</p>
          </div>
          <button onClick={() => setSetupOpen(true)} type="button">Begin setup <ArrowRight size={16} /></button>
        </section>
      )}

      <section className="metrics" aria-label="Library statistics">
        <Metric label="Controlled documents" value={overview?.documents} note="Across all categories" />
        <Metric label="Published binders" value={overview?.binders} note="Reproducible releases" />
        <Metric label="Reviews approaching" value={overview?.pendingReviews} note="Due in the next 30 days" attention />
        <Metric label="Category branches" value={overview?.categories} note="One primary home per document" />
      </section>

      <div className="dashboard-grid">
        <section className="panel getting-started-panel">
          <div className="panel-heading">
            <div>
              <p className="eyebrow">Build your library</p>
              <h2>Start with the structure</h2>
            </div>
            <span className="progress-label">0 of 3</span>
          </div>
          <div className="starter-list">
            {starterActions.map(({ icon: Icon, title, detail }, index) => (
              <button className="starter-row" key={title} onClick={index === 0 ? onImport : undefined} type="button">
                <span className="step-number">0{index + 1}</span>
                <span className="starter-icon"><Icon size={20} strokeWidth={1.7} /></span>
                <span className="starter-copy"><strong>{title}</strong><small>{detail}</small></span>
                <ArrowRight size={17} strokeWidth={1.8} />
              </button>
            ))}
          </div>
        </section>

        <section className="panel recent-panel">
          <div className="panel-heading">
            <div>
              <p className="eyebrow">Recent activity</p>
              <h2>Latest changes</h2>
            </div>
            <button className="text-button" type="button">View audit log</button>
          </div>
          <div className="empty-state">
            <div className="empty-document"><FileCheck2 size={26} strokeWidth={1.4} /></div>
            <h3>A clean beginning</h3>
            <p>Uploads, approvals, binder releases, and other controlled changes will be recorded here.</p>
            <button className="secondary-button" onClick={onImport} type="button"><Plus size={16} /> Import documents</button>
          </div>
        </section>
      </div>

      <footer className="workspace-note">
        <span className={overview?.stirlingConfigured ? "healthy" : "pending"} />
        Stirling-PDF {overview?.stirlingConfigured ? "is configured" : "will be connected through environment settings"}
      </footer>
      <SetupDialog
        onClose={() => setSetupOpen(false)}
        onComplete={onSetupComplete}
        open={setupOpen}
      />
    </div>
  );
}

type MetricProps = {
  label: string;
  value?: number;
  note: string;
  attention?: boolean;
};

function Metric({ label, value, note, attention = false }: MetricProps) {
  return (
    <article className="metric">
      <span>{label}</span>
      <strong className={attention && value ? "attention" : undefined}>{value ?? "—"}</strong>
      <small>{note}</small>
    </article>
  );
}
