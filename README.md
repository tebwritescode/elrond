# Elrond

Elrond is a self-hosted document library for preserving source files, managing
controlled PDF versions, and publishing reproducible professional binders.

The project is under active development. Its architecture keeps business rules,
storage, PDF processing, HTTP delivery, and the browser interface in separate
modules so each can evolve independently.

## Printable Binder

The Binder Studio generates one PDF from every latest PDF-ready document. The
output contains a page-numbered index, a full-page separator for each category,
one separator identifying every document, and every source page in deterministic
category and title order.
Source PDFs are checksum-verified before assembly and are never modified.

Custom document selection and ordering, binder templates, covers, bookmarks,
headers, footers, duplex blanks, and saved release history are future features.

## Development

Requirements:

- Rust 1.97 or newer
- Node.js 24 or newer
- An optional external Stirling-PDF instance

Copy `.env.example` to `.env`, then run the API and web client:

```text
scripts\dev-api.cmd
npm --prefix web run dev
```

The web client is available at `http://127.0.0.1:5173` and proxies API requests
to `http://127.0.0.1:3000`.

Local databases, uploads, generated documents, secrets, and backups are excluded
from version control.

Set both `ELROND_ADMIN_USERNAME` and `ELROND_ADMIN_PASSWORD` to create the first
administrator without the setup screen. Leave both unset or empty to retain the
interactive first-run setup. The password must contain 12 to 128 characters.

The library supports direct document uploads, nested folder-ZIP imports,
editable category trees, document reassignment and tags, inline PDF viewing, and
original-file downloads. ZIP entries whose contents do not match their extension
are skipped and reported without discarding valid entries in the archive.

## Docker

Build and run Elrond locally with Compose:

```text
docker compose up --build
```

The application listens on `http://localhost:3000` and stores its SQLite
database, immutable originals, and derivatives in the
`elrond-data` volume. Set `ELROND_SECURE_COOKIES=true` when the application is
served through HTTPS.

Docker Hub publication is handled by GitHub Actions. The repository must define
`DOCKER_USERNAME` and `DOCKER_PASSWORD` secrets; `DOCKER_PASSWORD` contains a
Docker Hub access token rather than the account password. While both independent
implementations remain in beta, this primary branch publishes amd64-only `beta`
and `<semver>-beta` images; it never publishes `latest`. Stable releases retain
amd64 and arm64 coverage. The canonical publishing contract is
`docs/publishing.md` on the GitHub `alt` branch.

## License

Elrond is available under the MIT License.
