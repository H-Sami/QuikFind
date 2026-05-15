# QuikFind

Fast, private, local desktop search and launcher for Windows.

Built with Rust, Tauri 2, React 18, TypeScript, Vite, Tailwind CSS, Tantivy, Nucleo, SQLite, notify, jwalk, and rayon.

## Features

- File and folder search backed by Tantivy
- Fuzzy filename matching with Nucleo
- App launcher results merged into the main search view
- Stable app cache keyed by app path
- Two-phase indexing: metadata first, then text content enrichment
- Clean full reindex that clears stale documents before rebuilding
- Real-time file watching for create, modify, delete, and rename events
- Glob-based exclusions shared by indexing and watching
- Configurable global hotkey
- Optional desktop typing trigger, disabled by default
- Search history recorded by the backend when items are opened
- Query caching only when the index is stable
- System tray integration, window state persistence, and autostart setting

## Quick Start

```bash
npm install
npm run tauri dev
```

## First Run

On first launch, QuikFind builds a clean index for the configured paths, or all local Windows drives when no paths are configured. Metadata indexing runs first so names and paths become searchable quickly. Content enrichment runs under the same supervised lifecycle, reloads the Tantivy reader after commits, and makes small text-file content searchable without restarting the app.

## Build

```bash
npm run tauri build
```

Installers are emitted under `src-tauri/target/release/bundle/`.

## Verification

```bash
npm run build
cd src-tauri
cargo test
cargo clippy --all-targets -- -D warnings
```

## Project Layout

```text
QuikFind/
|-- src/                  # React frontend
|-- src-tauri/src/        # Rust backend
|   |-- lib.rs            # Tauri setup and subsystem composition
|   |-- commands.rs       # Thin Tauri command handlers
|   |-- indexing.rs       # Indexing supervisor and lifecycle
|   |-- indexer.rs        # File traversal and document building
|   |-- search.rs         # Tantivy search mechanics and query cache
|   |-- watcher.rs        # Live filesystem deltas
|   |-- apps.rs           # OS app scanner and launcher
|   |-- settings.rs       # SQLite persistence
|   |-- hotkey.rs         # Hotkey validation and registration
|   `-- desktop_listener.rs
|-- src-tauri/Cargo.toml
|-- src-tauri/tauri.conf.json
`-- package.json
```

## License

MIT
