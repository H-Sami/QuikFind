# Changelog

## [1.0.1] — 2026-05-15

### Fixed

- **Stale search results during indexing (Bug 1).** Removed the cached `Searcher`
  snapshot — `perform_search` now calls `reader.searcher()` fresh on every request,
  so it always reflects the latest committed index data.
- **Cache poisoning during indexing (Bug 2).** The query cache is now skipped while
  indexing is active, preventing empty mid-index results from permanently replacing
  real results.
- **`stop_indexing` left index unsearchable (Bug 3).** `stop_indexing` now calls
  `finish_batch_index` after aborting, which commits buffered documents, reloads
  the reader, and clears stale cache entries.

## [1.0.0] — Initial release
