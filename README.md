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

## License

Elrond is available under the MIT License.
