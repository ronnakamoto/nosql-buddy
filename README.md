# NoSQLBuddy

A cross-platform MongoDB management studio for developers, SREs, and database engineers who work across local, staging, and production environments.

NoSQLBuddy connects to MongoDB, lets you browse data, run queries, build aggregations, translate SQL to MongoDB, inspect schema and indexes, and review performance — all from a native desktop app built with Tauri, Rust, React, and TypeScript.

## Features

- **Connection management** — Save connection profiles with secrets stored in the OS keychain. URIs and credentials are redacted from logs and UI responses.
- **Data browsing** — Query collections, paginate results, edit documents in place, and view JSON or table output.
- **Visual query builder** — Compose filters, projections, and sorts without writing raw JSON.
- **Aggregation editor** — Build and preview aggregation pipelines with syntax-aware JSON editing.
- **SQL to MongoDB** — Translate `SELECT`, `JOIN`, `GROUP BY`, `WHERE`, `ORDER BY`, and `LIMIT` statements into aggregation pipelines.
- **Schema and index analysis** — Infer schema shape, cardinality, and index usage from sampled documents.
- **Explain plan visualization** — Parse `explain` output into a navigable tree to diagnose slow queries.
- **Driver code generation** — Export queries and pipelines to Node.js, Python, Java, C#, Ruby, Rust, and the MongoDB shell.
- **Native desktop experience** — Built on Tauri v2 for a small footprint, native menus, and consistent shortcuts on macOS, Windows, and Linux.

## Tech stack

- **Frontend:** React 18, TypeScript, Vite, visx
- **Backend:** Rust, Tauri v2, Tokio
- **Database:** MongoDB driver for Rust (`mongodb` + `bson`)
- **Testing:** Playwright (frontend), Cargo (Rust unit + integration tests)

## Getting started

### Prerequisites

- [Node.js](https://nodejs.org/) 18+
- [Rust](https://www.rust-lang.org/tools/install) 1.77+
- A MongoDB instance (local or remote) for live features

### Installation

```bash
git clone https://github.com/ronnakamoto/nosql-buddy.git
cd nosql-buddy
npm install
```

### Development

Run the full Tauri dev environment (Rust backend + Vite frontend):

```bash
npm run tauri dev
```

Run the frontend alone against a mocked backend:

```bash
npm run dev
```

### Build

```bash
npm run build      # Production frontend build
npm run tauri build  # Native app bundle for the current platform
```

## Testing

```bash
# Frontend type check and production build
npm run build

# Frontend smoke test (Playwright)
npx playwright test

# Rust linting
cd src-tauri
cargo clippy --all-targets --all-features -- -D warnings

# Rust tests
cargo test --all-targets
```

## Security

- Connection secrets are stored in the OS keychain, not in plaintext files or settings.
- Passwords and URIs are redacted from error messages, logs, and IPC responses.
- Tauri capabilities are scoped to the minimum permissions required by the main window.

## Contributing

Contributions are welcome. Please open an issue or pull request with a clear description of the change, reproduction steps for bugs, and tests where possible.

## License

See the [LICENSE](./LICENSE) file for details.

## Acknowledgments

Built with [Tauri](https://tauri.app/), [React](https://react.dev/), [Vite](https://vitejs.dev/), and the MongoDB Rust driver.
