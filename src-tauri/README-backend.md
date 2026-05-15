# QuikFind Backend

Rust and Tauri 2 backend for local desktop search and app launching.

## Architecture

```text
src/
|-- main.rs              # Entry point
|-- lib.rs               # App setup and subsystem composition
|-- commands.rs          # Thin Tauri command handlers
|-- indexing.rs          # Single indexing lifecycle owner
|-- indexer.rs           # jwalk/rayon traversal and document construction
|-- search.rs            # Tantivy mechanics, reader reload, query cache
|-- watcher.rs           # notify-based live filesystem deltas
|-- apps.rs              # OS app scanner and launcher
|-- settings.rs          # SQLite settings, history, file metadata, app cache
|-- hotkey.rs            # Hotkey validation and atomic registration
|-- desktop_listener.rs  # Optional desktop typing listener
|-- platform.rs          # Platform helpers
`-- plugins.rs           # In-process plugin registry skeleton
```

## Indexing Lifecycle

`IndexingSupervisor` is the only source of truth for active indexing state. It owns the current task handle, cancellation signal, phase, status, progress emission, start/stop, and rebuild behavior.

Indexing has two supervised phases:

- `metadata`: walks configured paths and indexes path/name/metadata.
- `content`: revisits indexed text files and enriches Tantivy documents with bounded text content.

Every index mutation that should be visible to search uses the same path: commit, reload the Tantivy reader, and invalidate query caches. Search commands disable cache use while the supervisor reports an active phase.

`reindex_all` stops active indexing, stops the watcher, clears Tantivy, clears `indexed_files`, and starts a clean rebuild. The watcher is restarted after metadata indexing completes.

## Search

- File identity is a stable hash of normalized path.
- Re-indexing or watcher upserts replace the document with the same stable ID.
- Query cache keys include query, limit, offset, max-results, and content-search mode.
- Empty or whitespace-only queries return no app or file results.
- App results are cached in SQLite and merged into the main `search` command for non-empty queries.

## Watcher

The watcher classifies notify events into upserts and removals. Delete paths are preserved even when the file no longer exists. Rename pairs are classified before exclusion filtering, so moves into excluded locations remove old indexed paths and moves out of excluded locations upsert the new file. Create, modify, remove, and rename events commit, reload, and invalidate caches when they change the index. Exclusion patterns are shared with the indexer.

## Settings and Hotkeys

Settings persistence is validation-first. Hotkeys are parsed and registered before settings are saved. If registration fails, the previous hotkey remains active and settings are not persisted.

The desktop typing trigger is controlled by `enable_type_to_search` and defaults to disabled. The listener starts only when enabled and emits characters only for unmodified text keypresses while the foreground window is the Windows desktop.

## Commands

- `search`
- `open_path`
- `launch_app`
- `start_indexing`
- `stop_indexing`
- `reindex_all`
- `get_index_status`
- `get_settings`
- `update_settings`
- `get_history`
- `scan_apps_now`
- `get_plugins`
- `get_window_state`
- `save_window_state`

## Development

```bash
cd src-tauri
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
```

The repository ignores `src-tauri/.cargo/` intentionally because that directory is for local linker or toolchain overrides.
