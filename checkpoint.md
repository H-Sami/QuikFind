# checkpoint.md

End-of-session checkpoint for the QuikFind architecture repair.

Date: 2026-05-16  
Workspace: `C:\Users\PC\Desktop\QuikFind`

## Final Status

The repair is complete. The backend indexing lifecycle, reindex behavior, watcher correctness, search cache behavior, app launcher integration, hotkey persistence, Type to Search setting, frontend state flow, docs, version metadata, and CI have all been updated.

Final verification passed:

```bash
npm.cmd run build
cd src-tauri
cargo test
cargo clippy --all-targets -- -D warnings
```

Last observed test count: 16 Rust tests passed.

## Core Invariants Now Enforced

- `IndexingSupervisor` is the only source of truth for indexing activity.
- Phase 1 metadata indexing and phase 2 content enrichment are a single supervised lifecycle.
- Stopping indexing cancels the supervised lifecycle, including content enrichment.
- Reindex is a clean rebuild: stop active lifecycle, stop watcher, clear Tantivy, clear `indexed_files`, rebuild.
- Search cache is bypassed while indexing is active.
- Every index mutation that should affect search commits, reloads the Tantivy reader, and invalidates query cache.
- Watcher deltas preserve deleted paths and obey the same exclusion rules as indexing.
- File and app identity are stable normalized path hashes.
- Hotkey and settings updates validate first and persist only after side effects succeed.
- Type to Search is disabled by default and controlled by settings.
- The backend is the single writer for open/launch history.

## Final Architecture

### Backend Composition

`src-tauri/src/lib.rs` now composes subsystems:

- `SearchEngine`
- `SettingsDatabase`
- `FileWatcher`
- `AppScanner`
- `PluginRegistry`
- `IndexingSupervisor`
- `DesktopListener`
- active hotkey state

Setup no longer contains indexing lifecycle logic. Startup either starts a clean supervised rebuild when no indexed files exist, or starts the watcher against the existing index.

### Indexing Supervisor

New file: `src-tauri/src/indexing.rs`.

`IndexingSupervisor` owns:

- current task handle
- cancellation signal
- active status
- active phase
- progress emission
- incremental start
- rebuild start
- stop behavior

Important types:

- `IndexPhase`: `Idle`, `Metadata`, `Content`
- `IndexRequest`: `Incremental`, `Rebuild`
- `IndexingJob`: paths, request type, settings, app handle, search engine, database, watcher
- `IndexingSupervisor`: lifecycle owner

Lifecycle:

1. Stop watcher before indexing starts.
2. If rebuild, clear Tantivy and `indexed_files`.
3. Run metadata indexing through `Indexer::index_paths`.
4. Emit `index-phase1-complete`.
5. Restart watcher after metadata phase.
6. Run content enrichment through `Indexer::enrich_content`.
7. Return to idle status with current indexed count.

Cancellation:

- A shared `AtomicBool` is passed into indexing and enrichment.
- `stop_indexing` sets cancellation and awaits the lifecycle task.
- The old detached phase-2 task pattern is gone.

### Search Engine

`src-tauri/src/search.rs` now owns only Tantivy mechanics and query caching.

Key behavior:

- Removed internal indexing state.
- Removed unused `popular_cache`.
- Removed `fuzzy_threshold` path.
- Added `invalidate_caches()`.
- Added `commit_reload_invalidate()`.
- Cache keys include query, limit, offset, max results, and content-search mode.
- Deduplication now uses normalized path identity, not adjacent-only `dedup_by`.
- `index_document` replaces by stable ID before adding, preventing duplicates.

### Indexer

`src-tauri/src/indexer.rs` now:

- Accepts cancellation for metadata and content phases.
- Returns an `IndexingProgress` summary.
- Uses stable path-based file IDs.
- Commits through `commit_reload_invalidate`.
- Records directly indexed single files in SQLite.
- Exposes shared glob/exclusion helpers for watcher use.

### Watcher

`src-tauri/src/watcher.rs` was rewritten around classified event deltas.

Watcher behavior:

- Classifies notify events into upserts and removals.
- Keeps remove paths even if the path no longer exists.
- Handles rename `Both`, `From`, and `To` modes.
- Ignores directories for file upserts.
- Applies the same exclusion rules as indexing.
- Uses metadata plus bounded content mode for upserts.
- Commits, reloads, and invalidates caches only when a real change occurred.
- Stops cleanly before full reindex so it cannot fight the rebuild.

### App Launcher

`src-tauri/src/apps.rs` now:

- Uses stable app IDs derived from normalized path.
- Clears cached apps before scan results are reinserted.
- Deduplicates apps by stable ID.
- Exposes cached app reads for empty query so search does not trigger an expensive scan.

`src-tauri/src/commands.rs` merges app results into the main `search` command. The separate frontend `search_apps` path is no longer needed.

### Hotkey and Settings

`src-tauri/src/hotkey.rs` now:

- Validates hotkeys before registration.
- Rejects modifier-only shortcuts.
- Allows an empty hotkey to mean disabled.
- Registers the new hotkey before unregistering the old one.
- Leaves the old hotkey active if new registration fails.

`update_settings` in `commands.rs` now:

- Validates hotkey.
- Applies autostart and hotkey side effects before persistence.
- Rolls back side effects where possible if persistence fails.
- Updates in-memory settings only after save succeeds.
- Starts or stops Type to Search emission based on saved settings.

### Type to Search

`src-tauri/src/desktop_listener.rs` is now represented by `DesktopListener`.

Behavior:

- `enable_type_to_search` defaults to false.
- The listener starts only after the setting is enabled.
- Emission is gated by an atomic setting.
- Modifier combinations are ignored.
- Text comes from `rdev` event names instead of a hardcoded US keyboard map.
- Desktop detection is stricter.

Known limitation:

- `rdev::listen` has no unlisten API. After enabling Type to Search once, the listener thread remains alive, but emissions are disabled when the setting is off.

### Platform

`src-tauri/src/platform.rs` desktop detection now checks foreground window class names `Progman` and `WorkerW` instead of treating empty/untitled windows as desktop.

### Frontend

Search:

- `useSearch` calls main `search` for all queries, including empty query.
- Empty query can show cached app results without frontend special-casing.
- Search errors are logged and surfaced through toast.

Opening:

- `useOpenResult` launches `App` results through `launch_app`.
- File/folder results use `open_path`.
- History writes are no longer duplicated in the frontend.
- Open errors are no longer swallowed.

Settings:

- Settings are staged locally and saved deliberately.
- Reindex button calls `reindex_all`.
- Startup setting is persisted through `update_settings`; separate frontend `set_autostart` call was removed.
- Type to Search setting was added.
- Hotkey recorder ignores modifier-only keydown events and saves on explicit Save.
- Backend errors are shown to the user.

Results:

- Removed dead virtualization threshold and fixed keyboard scroll behavior through `lastNavMethodRef`.
- Removed unused density state.
- Click selects; double-click opens.
- Keyboard Enter still opens through shortcut handling.

Shortcuts:

- Arrow shortcut defaults are now ASCII strings: `ArrowUp`, `ArrowDown`.
- Old mojibake arrow shortcuts in local storage are migrated in `uiStore`.

## How Each Major Problem Was Fixed

### 1. Indexing lifecycle was broken

Fixed by adding `src-tauri/src/indexing.rs`.

Before:

- `run_indexing` spawned detached phase 2 content enrichment.
- `indexing_handle` only tracked phase 1.
- `AppState::is_indexing` and `SearchEngine::is_indexing` could diverge.
- Content commits did not reload or clear caches.

After:

- `IndexingSupervisor` owns lifecycle state and task handle.
- Metadata and content are one task.
- Cancellation reaches both phases.
- SearchEngine has no indexing flag.
- Commits use `commit_reload_invalidate`.
- Cache use is disabled from command layer while supervisor is active.
- Progress includes phase and does not reset file count to zero on idle.

### 2. Reindexing appended instead of rebuilding

Fixed in `commands::reindex_all` and `IndexingSupervisor::start`.

Before:

- UI called `start_indexing`.
- `reindex_all` cleared but did not restart.

After:

- UI calls `reindex_all`.
- Rebuild request stops existing lifecycle.
- Watcher stops.
- Tantivy index is cleared.
- `indexed_files` is cleared.
- Indexing restarts from selected/default paths.
- Watcher restarts after metadata phase.

### 3. File watcher correctness was broken

Fixed by rewriting `watcher.rs`.

Before:

- Remove events filtered with `p.exists()`.
- Create events did not set commit.
- Cache was not invalidated.
- Exclusions ignored.
- Rename behavior incomplete.

After:

- Deletes preserve non-existent paths.
- Create/modify/remove/rename are classified into upserts/removes.
- Directories are not indexed as files.
- Exclusions are applied.
- Real changes call commit, reload, invalidate.
- Rename old path is removed and new path is indexed.
- Watcher is stopped during full rebuild.

### 4. Search cache and query correctness needed cleanup

Fixed in `search.rs` and `models.rs`.

Before:

- `fuzzy_threshold` was ignored.
- Cache key omitted result-affecting settings.
- `popular_cache` was unused.
- Dedup only removed adjacent duplicate paths.
- SearchEngine duplicated indexing state.

After:

- Removed `fuzzy_threshold` from settings/types/path.
- Cache key includes all meaningful inputs.
- Removed `popular_cache`.
- Added `invalidate_caches`.
- Robust normalized path dedup.
- SearchEngine no longer tracks indexing.

### 5. App launcher was advertised but not integrated

Fixed in `apps.rs`, `commands.rs`, and frontend open/search hooks.

Before:

- Frontend never called app search in normal flow.
- App IDs were random UUIDs.
- App duplicates could accumulate forever.

After:

- Main `search` merges app results.
- App IDs are stable path hashes.
- Scans clear and replace cached apps.
- Empty query reads cached apps only.
- App results launch through `launch_app`.

### 6. Hotkey update was not atomic

Fixed in `hotkey.rs` and `commands::update_settings`.

Before:

- Old hotkey was unregistered before validating/registering new one.
- Settings could save despite hotkey registration failure.
- Frontend recorder accepted modifier-only shortcuts.

After:

- Hotkey validation rejects modifier-only values.
- New hotkey is registered before old one is removed.
- Settings save fails if hotkey registration fails.
- Frontend recorder ignores modifier-only keydown events.
- Errors are surfaced to the user.

### 7. Type to Search was unsafe

Fixed in `desktop_listener.rs`, `platform.rs`, settings, and UI.

Before:

- Always enabled.
- Could not be controlled by user.
- Used hardcoded US keyboard mapping.
- Treated untitled foreground windows as desktop.

After:

- New `enable_type_to_search` setting, default false.
- Listener emission is controlled by setting.
- Modifier combinations ignored.
- Uses event text from `rdev`.
- Desktop detection requires desktop class names.

### 8. Frontend state and UX cleanup

Fixed across frontend hooks, components, stores, and types.

Before:

- `density` stored but unused.
- Virtualization threshold impossible to hit with max 50 results.
- Keyboard nav did not reliably set keyboard nav method.
- Result click both selected and opened.
- Settings mixed immediate save/local save/apply.
- Open errors swallowed.
- History written twice.

After:

- Removed density.
- Removed dead virtualization.
- Keyboard nav sets `lastNavMethodRef`.
- Single click selects; double click opens.
- Settings save is explicit.
- Open/search/settings errors log and toast.
- Backend alone writes history.

### 9. Docs, config, and CI stale

Fixed in docs, version files, CI, and gitignore.

Before:

- Backend docs described missing commands/features and stale cache/batch details.
- package version was `0.1.0`.
- CI only checked Rust.
- `.gitignore` explanation around `src-tauri/.cargo/` was unclear.

After:

- README and backend README reflect current architecture.
- package version aligned to `1.0.2`.
- CI installs frontend dependencies and runs frontend build.
- CI runs clippy with `--all-targets -- -D warnings`.
- `.gitignore` states local Cargo overrides are ignored intentionally.
- Removed direct `uuid` dependency.

## Files Created

- `src-tauri/src/indexing.rs`
- `AGENTS.md`
- `ROADMAP.md`
- `checkpoint.md`

## Files Edited

Backend:

- `src-tauri/src/apps.rs`
- `src-tauri/src/commands.rs`
- `src-tauri/src/desktop_listener.rs`
- `src-tauri/src/error.rs`
- `src-tauri/src/hotkey.rs`
- `src-tauri/src/indexer.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/src/models.rs`
- `src-tauri/src/platform.rs`
- `src-tauri/src/search.rs`
- `src-tauri/src/settings.rs`
- `src-tauri/src/watcher.rs`
- `src-tauri/Cargo.toml`
- `src-tauri/Cargo.lock`
- `src-tauri/README-backend.md`

Frontend:

- `src/App.tsx`
- `src/components/ResultsList.tsx`
- `src/components/SettingsModal.tsx`
- `src/hooks/constants.ts`
- `src/hooks/useKeyboardShortcuts.ts`
- `src/hooks/useOpenResult.ts`
- `src/hooks/useSearch.ts`
- `src/store.ts`
- `src/stores/shortcutDefaults.ts`
- `src/stores/uiStore.ts`
- `src/types.ts`
- `src/utils/shortcuts.ts`

Repo/config/docs:

- `.github/workflows/ci.yml`
- `.gitignore`
- `README.md`
- `package.json`
- `package-lock.json`

## Files Deleted

No tracked files were removed from the repository. Several files were rewritten in place to replace broken implementations.

## Tests Added

Rust unit coverage added for:

- Stable app IDs.
- App deduplication.
- Search cache key inputs.
- Search cache invalidation.
- Robust path deduplication.
- Stable file ID normalization.
- Hotkey validation.
- Watcher delete path preservation.
- Watcher create filtering and exclusions.
- Watcher rename classification.

## Verification Commands Passed

From repo root:

```bash
npm.cmd run build
```

From `src-tauri`:

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

## Remaining Risks

- `rdev::listen` cannot be stopped after it starts. The implementation keeps Type to Search disabled by default and gates emissions off when disabled, but the OS listener thread remains alive after first enablement.
- Watcher logic has focused unit tests for event classification. A future integration test should exercise watcher deltas against a real temp Tantivy index and SQLite database.
- A manual Windows smoke test is still valuable for global hotkey conflicts, real notify rename behavior, app launch quirks, and Type to Search desktop detection.
