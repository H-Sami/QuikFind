# AGENTS.md

Guidance for future agents working in this repository.

## Project

QuikFind is a private local desktop search and launcher for Windows.

Stack:
- Frontend: React 18, TypeScript, Vite, Tailwind, Zustand
- Backend: Tauri 2, Rust
- Search: Tantivy and Nucleo
- Storage: SQLite through rusqlite
- File watching: notify
- Traversal: jwalk and rayon

## Current Architecture Rules

- `src-tauri/src/lib.rs` composes subsystems only. Keep lifecycle logic out of setup.
- `src-tauri/src/commands.rs` should stay thin. Commands validate, call subsystems, and translate errors.
- `src-tauri/src/indexing.rs` owns indexing lifecycle. Do not reintroduce indexing state elsewhere.
- `src-tauri/src/search.rs` owns Tantivy mechanics, reader reload, and query cache behavior.
- `src-tauri/src/indexer.rs` builds file documents and performs traversal/enrichment work.
- `src-tauri/src/watcher.rs` owns live filesystem deltas and must use the same exclusion rules as indexing.
- `src-tauri/src/settings.rs` owns persistence only.
- The frontend should not duplicate backend state or pretend failed backend operations succeeded.

## Core Invariants

- There is exactly one source of truth for indexing activity: `IndexingSupervisor`.
- Metadata indexing and content enrichment are one supervised lifecycle.
- `stop_indexing` must cancel all active indexing phases.
- Reindex means a clean rebuild, never append.
- Every visible index mutation must commit, reload the Tantivy reader, and invalidate query caches.
- Search cache must not be used while any indexing phase is active.
- Watcher events must preserve removed paths even when those paths no longer exist.
- Watcher updates must respect configured exclusion patterns.
- File identity is a stable normalized path hash.
- App identity is a stable normalized path hash.
- Settings are persisted only after validation and side effects succeed.
- Hotkey registration failure must leave the previous hotkey active.
- Type to Search must remain user-controlled and disabled by default.
- History is written by the backend open/launch commands.

## Required Verification

Run these before handing off changes:

```bash
npm.cmd run build
cd src-tauri
cargo test
cargo clippy --all-targets -- -D warnings
```

## Notes

- The repository intentionally ignores `src-tauri/.cargo/` for local linker/toolchain overrides.
- Avoid adding dependencies unless they clearly reduce complexity or fix correctness.
- Preserve unrelated user changes.
- Prefer focused tests for pure logic: IDs, cache keys, event classification, validation helpers.
