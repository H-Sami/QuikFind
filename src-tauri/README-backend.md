# QuikFind Backend

Blazing-fast, private, cross-platform desktop search & launcher built with Rust and Tauri 2.

## Architecture

```
src/
├── main.rs         # Entry point (calls lib::run)
├── lib.rs          # Tauri app setup, state init, command registration
├── models.rs       # Data models, Tantivy schema, constants
├── error.rs        # Error types (thiserror)
├── settings.rs     # SQLite-backed settings, history, file metadata
├── search.rs       # Tantivy search engine + nucleo fuzzy matching
├── indexer.rs      # Parallel file walker (jwalk + rayon)
├── watcher.rs      # Real-time file watching (notify)
├── apps.rs         # OS app scanner (Windows/macOS/Linux)
├── commands.rs     # All Tauri command handlers
└── plugins.rs      # WASM plugin skeleton (Wasmtime)
```

## Prerequisites

- Rust 1.77+
- Tauri 2 CLI: `cargo install tauri-cli --version "^2"`
- Platform-specific dependencies (see [Tauri docs](https://v2.tauri.app/start/prerequisites/))

## Development

```bash
# Run in dev mode (with hot-reload frontend)
cargo tauri dev

# Build for production
cargo tauri build
```

## Backend-only Build & Test

```bash
cd src-tauri
cargo build
cargo test
cargo clippy
```

## Configuration

Settings database: `~/.config/quikfind/quikfind.db`
Tantivy index: `~/.config/quikfind/index/`
Plugin directory: `~/.config/quikfind/plugins/`

## Key Design Decisions

### Search Engine
- Tantivy with MmapDirectory for memory-mapped indexes
- Nucleo for fuzzy filename matching (3x weighted vs content)
- Recency boost for files modified within last 24h
- LRU query cache (128 entries)

### Indexing
- jwalk + rayon for parallel directory traversal
- 1000-document batch commits
- Content extraction limited to 50KB per text file
- Glob-based exclusion patterns (node_modules, .git, etc.)

### File Identity
- Document ID = blake3(path:size:mtime) for dedup

### Performance Targets
- Search latency: <50ms for 1M+ files
- Indexing: >50,000 files/min
- Memory: <50MB idle
- Binary: <15MB stripped

## Tauri Commands

See `commands.rs` for all commands matching the frontend's expected API:
- `search`, `get_preview`, `open_path`
- `start_indexing`, `stop_indexing`, `get_index_status`
- `get_settings`, `update_settings`
- `search_apps`, `launch_app`
- `get_history`, `add_to_history`
- `reindex_all`, `scan_apps_now`, `get_plugins`

## Plugin System (WASM)

Plugin skeleton uses Wasmtime. Built-in `HelloPlugin` demonstrates the API.
WASM plugins go in `~/.config/quikfind/plugins/` and are loaded at startup.

## Benchmarks

```bash
cargo bench
```
