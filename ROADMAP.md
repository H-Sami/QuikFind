# ROADMAP.md

QuikFind is now past the lifecycle repair. The next work should build on the corrected architecture, not reopen the old split-state design.

## Completed In This Repair

- Single indexing supervisor with owned task handle, cancellation, phase/status, and progress emission.
- Clean reindex command that stops active work, clears the index and metadata table, then rebuilds.
- Watcher deltas for create, modify, remove, and rename with correct commit/reload/cache invalidation.
- Stable file and app identities.
- Main search now includes app results.
- Hotkey changes validate and register before settings persistence.
- Type to Search is optional and disabled by default.
- Frontend settings flow is explicit and failure-aware.
- Backend owns history writes.
- Docs, versions, and CI were updated.

## Near-Term Follow-Up

- Manual smoke test in a packaged or dev Tauri window:
  - First launch indexing.
  - Stop during metadata phase.
  - Stop during content phase.
  - Reindex all.
  - File create, modify, delete, and rename in a watched directory.
  - App search and launch.
  - Hotkey update failure path.
  - Type to Search enabled and disabled.
- Add an integration-style test harness for temporary Tantivy indexes and SQLite databases.
- Add watcher tests that exercise `process_pending` against a temp index/database, not only event classification.
- Add frontend tests for settings save failures and open-result app/file branching if a test runner is introduced.

## Product Direction

- Keep first screen as the actual launcher/search UI.
- Keep empty-query behavior fast. It should use cached app/history data only.
- Consider recent/frequent items for empty query, but keep history ownership in the backend.
- Content search should stay bounded and predictable. Avoid expensive extraction in the watcher unless deliberately designed.
- Plugin functionality is currently a registry skeleton. Do not advertise WASM or external plugin execution unless implemented.

## Engineering Direction

- Keep `IndexingSupervisor` as the only owner of indexing lifecycle state.
- Keep `SearchEngine` focused on Tantivy operations and caches.
- Keep watcher and indexer sharing exclusion logic.
- Avoid detached background tasks that mutate the index.
- Add abstractions only when they remove real complexity.
- Prefer path-stable identities and replace-by-ID writes over append-style indexing.

## CI

CI now runs:

```bash
npm ci
npm run build
cd src-tauri
cargo build --release
cargo clippy --all-targets -- -D warnings
cargo test
```

Keep CI Windows-compatible and fast.
