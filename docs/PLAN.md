# Elrond Project Plan

## Product

Elrond is a self-hosted document library for preserving source files, managing
controlled PDF versions, and publishing reproducible professional binders.

The initial deployment targets a single organization and local accounts with
admin, editor, reviewer, and viewer roles. The application will be distributed
as one Docker image with SQLite and local-volume storage. Stirling-PDF remains
an independently deployed service configured through environment variables.

## Research Outcome

No mature Rust and React document-management project is suitable as Elrond's
foundation. Elrond will therefore be a clean MIT-licensed implementation while
adopting proven patterns from:

- Paperless-ngx for ingestion, metadata, OCR, filtering, and archive workflows.
- Papra for immutable originals, SHA-256 deduplication, responsive archive UX,
  and portable deployment.
- Carbon Design System for information-dense tables, filtering, batch actions,
  loading states, and responsive behavior.
- WAI-ARIA Authoring Practices and WCAG 2.2 for accessible interaction.
- Stirling-PDF for authenticated PDF processing and pipelines.
- PDF.js for in-browser PDF rendering.

Electron is not part of the initial web application. The React client can be
wrapped in a desktop shell later without changing the server architecture.

## Architecture

Elrond is a modular monorepo producing one application image:

```text
elrond/
|-- crates/
|   |-- domain/           # Entities and workflow invariants
|   |-- application/      # Use cases and service interfaces
|   |-- infrastructure/   # SQLite, filesystem, search, and Stirling
|   |-- api/              # Axum routes, authentication, HTTP contracts
|   `-- server/           # Composition root and application binary
|-- web/
|   |-- src/app/          # Routing and application shell
|   |-- src/features/     # Documents, categories, binders, and admin
|   |-- src/components/   # Elrond design system
|   `-- src/lib/          # API client and shared utilities
|-- migrations/
|-- tests/
|-- docs/
`-- .github/workflows/
```

Core technologies:

- Rust, Axum, Tokio, and SQLx.
- SQLite in WAL mode.
- React, strict TypeScript, and Vite.
- Accessible owned components based on proven headless primitives.
- TanStack Query and TanStack Table for server state and library views.
- PDF.js for document viewing and annotation coordinates.
- A mounted `/data` volume for persistent state.
- An external Stirling-PDF instance configured with `STIRLING_URL` and an
  optional `STIRLING_API_KEY`.
- Persisted background jobs in SQLite.
- One Rust process serving the API, built frontend, and background work.

Domain code does not depend directly on Axum, SQLite, Stirling-PDF, or filesystem
implementations. This keeps future PostgreSQL, object storage, and alternate PDF
processors possible without rewriting business rules.

## Document Model

- Original files remain byte-for-byte immutable.
- Each upload receives a SHA-256 content checksum.
- Office files and images retain their source and receive a generated PDF copy.
- PDF is the canonical viewing, editing, and distribution format.
- Replacements create immutable document versions.
- Lifecycle states are draft, in review, published, superseded, and archived.
- Published versions cannot be silently replaced.
- Documents have one primary hierarchical category and multiple tags.
- Category relationships prevent cycles.
- Metadata and extracted content are indexed with SQLite FTS5.
- Scanned documents can be OCR processed through Stirling-PDF.
- Annotations are non-destructive overlays stored separately from the PDF.
- Review and expiration status appears on the dashboard.
- Audit records are append-only.

## ZIP Hierarchy Import

Elrond will accept ZIP archives as a bulk-ingestion source:

- Every folder becomes a category.
- Every nested folder becomes a child category.
- Supported files are imported into their containing category.
- Root-level files are placed in an explicitly selected or generated category.
- Matching sibling category names are reused rather than duplicated.
- Originals and folder-relative provenance are preserved.
- The operation is resumable and reports imported, skipped, duplicate, and
  unsupported files.
- Import validates normalized paths, expanded size, compression ratio, entry
  count, nesting depth, MIME type, and individual file size.
- Path traversal, symlinks, archive bombs, encrypted entries, and partial
  category trees from failed transactions are rejected.

## Binder System

A binder contains an ordered tree of sections, subsections, documents, and
generated pages. Every release snapshots:

- Pinned published document-version IDs.
- Original and derivative checksums.
- Section ordering.
- Template version.
- Cover and separator settings.
- Pagination configuration.
- Generated output checksum.

Professional binder output includes:

- Configurable front cover.
- Nested section and category separator pages.
- Clickable table of contents.
- Alphabetical and structural indexes.
- PDF bookmarks.
- Page labels and page numbers.
- Headers and footers.
- Optional blank pages for duplex printing.
- Binder release history.
- Rebuild and compare-before-release workflow.

Elrond generates the cover, separators, table of contents, and index; Stirling
merges and processes source PDFs; Elrond validates the returned file signature
and applies final outline metadata where needed. Stirling responses are treated
as untrusted because downstream pipeline failures can return an empty successful
HTTP response.

## Frontend Direction

The interface uses an editorial workspace style rather than a generic card
dashboard:

- Warm paper surfaces and restrained accent colors.
- Strong editorial typography and a compact professional tool hierarchy.
- Persistent category tree on desktop.
- Global search and upload actions.
- Full-width sortable document table with saved view state.
- Thumbnail view where visual browsing is useful.
- Document workspace with PDF navigation, metadata, versions, and comments.
- Three-pane binder builder with source library, outline, and properties/preview.
- Drag-and-drop with equivalent keyboard movement controls.
- Task-oriented dashboard for drafts, reviews, expiration, failed jobs, and
  recent changes.
- Mobile layouts stack panes instead of compressing desktop controls.
- Light and dark themes.
- Skeleton loading states, clear empty states, and recovery actions.
- Command palette and documented keyboard shortcuts.
- Undo for safe reversible actions.

The interface avoids excessive cards, hidden hover-only actions, low-contrast
text, unlabeled icon controls, and animation that delays work.

## Live Development

Elrond stays open in the browser while it is built:

- Vite HMR applies React and CSS updates without a full reload.
- `cargo-watch` rebuilds and restarts the Rust API.
- The frontend detects API restarts and reconnects automatically.
- Development data is persistent and excluded from Git.
- Migrations preserve test documents.
- Features are built as working end-to-end vertical slices.
- Work continues autonomously and pauses only when a product decision is needed.

Visible implementation order:

1. Application shell, design system, navigation, and first-run setup.
2. Document upload and persistent immutable storage.
3. Category tree, tags, library views, full-text search, and ZIP import.
4. PDF conversion, OCR, and browser viewer.
5. Version workflow and annotation overlays.
6. Dashboard and review status.
7. Binder designer and live structure preview.
8. Professional binder generation.
9. Import, export, verified backup, audit, and administration.
10. Accessibility, security, performance, and release hardening.

## Quality Targets

- WCAG 2.2 AA.
- Complete keyboard operation and visible unobscured focus.
- Minimum target-size compliance and no color-only status indicators.
- Largest Contentful Paint at or below 2.5 seconds.
- Interaction to Next Paint at or below 200 milliseconds.
- Cumulative Layout Shift at or below 0.1.
- Responsive tests at desktop, tablet, and phone widths.
- Performance tests with 10,000 documents.
- Pagination or virtualization for large collections.
- Progressive PDF page rendering.

Rust enforcement includes formatting, Clippy with warnings denied, unit and
integration tests, migration tests, explicit domain errors, and dependency
auditing. Frontend enforcement includes strict TypeScript, consistent linting
and formatting, feature modules, component tests, Storybook, Playwright,
automated accessibility checks, and visual regression tests.

## Security And Privacy

- No personal names or real email addresses in source, examples, documentation,
  or commit metadata.
- Git uses the public account name with a provider-generated noreply identity.
- No credentials are passed through process arguments when avoidable.
- Environment files, databases, uploads, generated PDFs, keys, and tokens are
  excluded from Git.
- Only sanitized environment examples are committed.
- Secret and dependency scans run locally and in CI.
- Public history is scanned before the first push.
- MIME types are verified from content and filenames never control storage paths.
- Upload limits, timeouts, extraction limits, and PDF job limits are enforced.
- Passwords use Argon2id.
- Sessions use opaque server-side tokens and secure HTTP-only cookies.
- CSRF protection, security headers, and rate limiting are enabled.
- Logs redact credentials and document contents.
- Portable backups contain a manifest and checksums.

## Git, Releases, And Publishing

Development uses small Conventional Commits. Semantic-version milestones are:

- `v0.1.0`: architecture, design system, authentication, and deployment.
- `v0.2.0`: individual upload and browsable document catalog.
- `v0.3.0`: persistent PDF conversion jobs and Stirling-PDF integration.
- `v0.4.0`: basic categorized printable binder generation.
- `v0.5.0`: custom binder designer, ordering, templates, and saved releases.
- `v0.6.0`: audit, import/export, backup, and restore.
- `v0.9.0`: security, accessibility, migration, and performance hardening.
- `v1.0.0`: complete supported release.

GitHub Actions run Rust and frontend quality gates, multi-stage Docker builds,
image scanning, and SBOM generation. While the two independent implementations
are compared, this primary `main` branch publishes only the `beta` moving tag and
`<semver>-beta` releases. It never publishes `latest`. The shared branch and tag
contract is maintained in `docs/publishing.md` on the GitHub `alt` branch.

The source repository is published to Gitea and GitHub. Docker images are
published to Docker Hub using repository secrets; credential values are never
stored in the repository.
