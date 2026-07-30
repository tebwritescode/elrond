# Elrond

Elrond is a self-hosted document library for preserving source files, managing
controlled PDF versions, and publishing reproducible professional binders.

The project is under active development. Its architecture keeps business rules,
storage, PDF processing, HTTP delivery, and the browser interface in separate
modules so each can evolve independently.

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

## Docker

Build and run Elrond locally with Compose:

```text
docker compose up --build
```

The application listens on `http://localhost:3000` and stores its SQLite
database, immutable originals, derivatives, and generated binders in the
`elrond-data` volume. Set `ELROND_SECURE_COOKIES=true` when the application is
served through HTTPS.

Docker Hub publication is handled by GitHub Actions. The repository must define
`DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN` secrets. Pushes to `main` publish the
`edge` tag; semantic-version tags publish version aliases and `latest`.

## License

Elrond is available under the MIT License.
