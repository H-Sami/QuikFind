use crate::error::{QuikFindError, Result};
use crate::models::{compute_file_id, is_text_extension, FileEntry, ResultKind};
use crate::search::SearchEngine;
use crate::settings::SettingsDatabase;
use globset::{Glob, GlobSet, GlobSetBuilder};
use jwalk::WalkDir;
use rayon::prelude::*;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// Controls whether file content is read during indexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexMode {
    /// Index only path, name, size, mtime, kind. No content extraction.
    MetadataOnly,
    /// Extract and index text content for text-extension files.
    ContentEnrichment,
}

/// Files per streaming chunk. Balances memory (~80 MB peak) against traversal latency.
const CHUNK_SIZE: usize = 20_000;

/// Tantivy is committed every this many chunks (~60k docs). Fewer commits = fewer
/// segment flushes = faster indexing. Final commit always happens at end of traversal.
const COMMITS_PER_FLUSH: u32 = 3;

/// Progress events are throttled to this interval to avoid flooding the IPC channel.
const PROGRESS_INTERVAL_MS: u64 = 500;

struct IndexingContext<'a> {
    exclude_set: &'a GlobSet,
    errors: &'a parking_lot::Mutex<Vec<String>>,
    files_indexed: &'a AtomicU64,
    total_files: &'a AtomicU64,
    last_progress: &'a parking_lot::Mutex<Instant>,
    progress_tx: &'a mpsc::UnboundedSender<crate::models::IndexingProgress>,
    cancel: &'a AtomicBool,
}

pub struct Indexer {
    search_engine: Arc<SearchEngine>,
    db: Arc<RwLock<SettingsDatabase>>,
}

impl Indexer {
    pub const fn new(search_engine: Arc<SearchEngine>, db: Arc<RwLock<SettingsDatabase>>) -> Self {
        Self { search_engine, db }
    }

    /// Phase 1: metadata-only indexing. Streams directory entries directly into the processing
    /// pipeline in chunks of [`CHUNK_SIZE`] without collecting the full tree into memory.
    /// When complete, the searcher is reloaded so results are immediately searchable.
    ///
    /// # Errors
    /// Returns an error if the glob set cannot be built.
    pub async fn index_paths(
        &self,
        paths: Vec<String>,
        excluded_patterns: &[String],
        progress_tx: mpsc::UnboundedSender<crate::models::IndexingProgress>,
        cancel: Arc<AtomicBool>,
    ) -> Result<crate::models::IndexingProgress> {
        let start = Instant::now();
        let total_files = AtomicU64::new(0);
        let files_indexed = AtomicU64::new(0);
        let errors = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let last_progress = Arc::new(parking_lot::Mutex::new(Instant::now()));

        let exclude_set = build_glob_set(excluded_patterns)?;

        let ctx = IndexingContext {
            exclude_set: &exclude_set,
            errors: &errors,
            files_indexed: &files_indexed,
            total_files: &total_files,
            last_progress: &last_progress,
            progress_tx: &progress_tx,
            cancel: &cancel,
        };

        progress_tx
            .send(crate::models::IndexingProgress {
                files_indexed: 0,
                total_files: 0,
                current_file: None,
                errors: Vec::new(),
            })
            .ok();

        let mut chunks_since_commit = 0u32;

        for path_str in prioritize_paths(paths) {
            if cancel.load(Ordering::Relaxed) {
                return Err(QuikFindError::Cancelled);
            }

            let path = Path::new(&path_str);
            if !path.exists() {
                warn!("Index path does not exist: {}", path_str);
                continue;
            }

            if path.is_file() {
                if let Err(e) = self
                    .index_file(
                        path,
                        ctx.exclude_set,
                        ctx.files_indexed,
                        IndexMode::MetadataOnly,
                    )
                    .await
                {
                    error!("Failed to index file {}: {}", path_str, e);
                }
                total_files.fetch_add(1, Ordering::Relaxed);
                continue;
            }

            // Streaming traversal — no .collect() on the walker
            let mut chunk = Vec::with_capacity(CHUNK_SIZE);
            for entry in WalkDir::new(path)
                .skip_hidden(false)
                .follow_links(false)
                .into_iter()
            {
                let Ok(entry) = entry else { continue };
                if cancel.load(Ordering::Relaxed) {
                    return Err(QuikFindError::Cancelled);
                }
                let entry_path = entry.path();
                if is_excluded(&entry_path, ctx.exclude_set) {
                    continue;
                }
                if entry.metadata().is_err() {
                    continue;
                }
                // All files pass — binaries are indexed by name; only content extraction is gated.

                total_files.fetch_add(1, Ordering::Relaxed);
                chunk.push(entry);

                if chunk.len() == CHUNK_SIZE {
                    let batch = std::mem::take(&mut chunk);
                    self.process_chunk(
                        &batch,
                        &ctx,
                        &mut chunks_since_commit,
                        IndexMode::MetadataOnly,
                    )
                    .await;
                }
            }
            if !chunk.is_empty() {
                self.process_chunk(
                    &chunk,
                    &ctx,
                    &mut chunks_since_commit,
                    IndexMode::MetadataOnly,
                )
                .await;
            }
        }

        self.search_engine.commit_reload_invalidate().await?;

        let elapsed = start.elapsed();
        let indexed_count = files_indexed.load(Ordering::Relaxed);
        #[allow(clippy::cast_precision_loss)]
        let files_per_min = indexed_count as f64 / elapsed.as_secs_f64() * 60.0;
        info!(
            "Indexing complete: {indexed_count} files in {elapsed:?} ({files_per_min:.0} files/min)",
        );

        let summary = crate::models::IndexingProgress {
            files_indexed: indexed_count,
            total_files: total_files.load(Ordering::Relaxed),
            current_file: None,
            errors: errors.lock().clone(),
        };

        progress_tx.send(summary.clone()).ok();

        Ok(summary)
    }

    /// Processes a single chunk of directory entries: parallel entry building, Tantivy indexing,
    /// batch SQLite writes, and periodic Tantivy commits.
    async fn process_chunk(
        &self,
        chunk: &[jwalk::DirEntry<((), ())>],
        ctx: &IndexingContext<'_>,
        chunks_since_commit: &mut u32,
        mode: IndexMode,
    ) {
        if ctx.cancel.load(Ordering::Relaxed) {
            return;
        }

        // Phase 1: CPU-parallel processing (no lock held)
        let file_entries: Vec<_> = chunk
            .par_iter()
            .filter_map(|entry| {
                entry.metadata().ok()?;
                process_entry(entry, mode).ok()
            })
            .collect();

        // Phase 2: Index documents into Tantivy (in-memory, no DB)
        for fe in &file_entries {
            match self.search_engine.index_document(fe) {
                Ok(()) => {
                    ctx.files_indexed.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => ctx.errors.lock().push(format!("{}: {e}", fe.path)),
            }
        }

        // Phase 3: Single transaction for all DB writes
        {
            let db = self.db.read().await;
            let doc_ids: Vec<String> = file_entries
                .iter()
                .map(|fe| compute_file_id(&fe.path))
                .collect();
            let batch: Vec<_> = file_entries
                .iter()
                .zip(doc_ids.iter())
                .map(|(fe, doc_id)| (fe.path.as_str(), fe.size, fe.modified, doc_id.as_str()))
                .collect();
            db.batch_record_indexed_files(&batch).ok();
        }

        // Periodic Tantivy commit
        *chunks_since_commit += 1;
        if *chunks_since_commit >= COMMITS_PER_FLUSH {
            self.search_engine.commit_reload_invalidate().await.ok();
            *chunks_since_commit = 0;
        }

        // Throttled progress reporting
        {
            let mut last = ctx.last_progress.lock();
            if last.elapsed() >= Duration::from_millis(PROGRESS_INTERVAL_MS) {
                ctx.progress_tx
                    .send(crate::models::IndexingProgress {
                        files_indexed: ctx.files_indexed.load(Ordering::Relaxed),
                        total_files: ctx.total_files.load(Ordering::Relaxed),
                        current_file: file_entries.last().map(|r| r.path.clone()),
                        errors: ctx.errors.lock().clone(),
                    })
                    .ok();
                *last = Instant::now();
            }
        }
    }

    /// Indexes a single file entry. Used for individual file paths passed directly (not via
    /// directory traversal).
    async fn index_file(
        &self,
        path: &Path,
        exclude_set: &GlobSet,
        files_indexed: &AtomicU64,
        mode: IndexMode,
    ) -> Result<()> {
        if is_excluded(path, exclude_set) {
            return Ok(());
        }

        let metadata = std::fs::metadata(path).map_err(QuikFindError::Io)?;
        let file_entry = build_file_entry(path, &metadata, mode);
        self.search_engine
            .index_document(&file_entry)
            .map_err(|e| QuikFindError::Generic(format!("Index doc: {e}")))?;
        let db = self.db.read().await;
        let doc_id = compute_file_id(&file_entry.path);
        db.record_indexed_file(
            &file_entry.path,
            file_entry.size,
            file_entry.modified,
            &doc_id,
        )?;
        files_indexed.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Phase 2: content enrichment. Reads all already-indexed paths from SQLite, filters to
    /// text files, re-extracts content, and replaces the Tantivy documents so that content
    /// becomes searchable. Runs as a background task and does not block the user.
    ///
    /// # Errors
    /// Returns an error if database queries or Tantivy operations fail.
    pub async fn enrich_content(
        &self,
        progress_tx: mpsc::UnboundedSender<crate::models::IndexingProgress>,
        cancel: Arc<AtomicBool>,
    ) -> Result<crate::models::IndexingProgress> {
        let all_files = {
            let db = self.db.read().await;
            db.get_all_indexed_files()
                .map_err(|e| QuikFindError::Generic(format!("DB query: {e}")))? // REASON: error type is String; .to_string() would apply to the String itself
        };

        // Filter to text-extension files only
        let text_files: Vec<_> = all_files
            .into_iter()
            .filter(|(path, _, _, _)| is_text_extension(path))
            .collect();

        if text_files.is_empty() {
            info!("No text files to enrich");
            let summary = crate::models::IndexingProgress {
                files_indexed: 0,
                total_files: 0,
                current_file: None,
                errors: Vec::new(),
            };
            progress_tx.send(summary.clone()).ok();
            return Ok(summary);
        }

        info!("Enriching content for {} text files", text_files.len());

        let total = text_files.len() as u64;
        let files_indexed = AtomicU64::new(0);
        let errors = parking_lot::Mutex::new(Vec::<String>::new());
        let last_progress = parking_lot::Mutex::new(Instant::now());
        let mut chunks_since_commit = 0u32;

        for chunk in text_files.chunks(CHUNK_SIZE) {
            if cancel.load(Ordering::Relaxed) {
                return Err(QuikFindError::Cancelled);
            }

            // Parallel content extraction
            let entries: Vec<_> = chunk
                .par_iter()
                .filter_map(|(path, _, _, _)| {
                    let p = Path::new(path);
                    let meta = std::fs::metadata(p).ok()?;
                    let entry = build_file_entry(p, &meta, IndexMode::ContentEnrichment);
                    // Only keep files where content was actually extracted
                    if entry.content.is_some() {
                        Some(entry)
                    } else {
                        None
                    }
                })
                .collect();

            // Delete old + add new Tantivy documents
            for entry in &entries {
                if let Err(e) = self.search_engine.update_document(entry) {
                    errors.lock().push(format!("{}: {e}", entry.path));
                }
                files_indexed.fetch_add(1, Ordering::Relaxed);
            }

            // Batch-update SQLite doc_ids (may have changed if file metadata changed)
            {
                let db = self.db.read().await;
                let doc_ids: Vec<String> =
                    entries.iter().map(|fe| compute_file_id(&fe.path)).collect();
                let batch: Vec<_> = entries
                    .iter()
                    .zip(doc_ids.iter())
                    .map(|(fe, doc_id)| (fe.path.as_str(), fe.size, fe.modified, doc_id.as_str()))
                    .collect();
                db.batch_record_indexed_files(&batch).ok();
            }

            chunks_since_commit += 1;
            if chunks_since_commit >= COMMITS_PER_FLUSH {
                self.search_engine.commit_reload_invalidate().await.ok();
                chunks_since_commit = 0;
            }

            // Throttled progress
            {
                let mut last = last_progress.lock();
                if last.elapsed() >= Duration::from_millis(PROGRESS_INTERVAL_MS) {
                    progress_tx
                        .send(crate::models::IndexingProgress {
                            files_indexed: files_indexed.load(Ordering::Relaxed),
                            total_files: total,
                            current_file: entries.last().map(|r| r.path.clone()),
                            errors: errors.lock().clone(),
                        })
                        .ok();
                    *last = Instant::now();
                }
            }
        }

        self.search_engine.commit_reload_invalidate().await?;
        info!("Content enrichment complete for {} files", total);
        let summary = crate::models::IndexingProgress {
            files_indexed: files_indexed.load(Ordering::Relaxed),
            total_files: total,
            current_file: None,
            errors: errors.lock().clone(),
        };
        progress_tx.send(summary.clone()).ok();
        Ok(summary)
    }
}

pub(crate) fn build_glob_set(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        match Glob::new(pattern) {
            Ok(glob) => {
                builder.add(glob);
            }
            Err(e) => {
                warn!("Invalid glob pattern '{}': {}", pattern, e);
            }
        }
    }
    builder
        .build()
        .map_err(|e| QuikFindError::Generic(format!("GlobSet build: {e}")))
}

pub(crate) fn is_excluded(path: &Path, exclude_set: &GlobSet) -> bool {
    let path_str = path.to_string_lossy();
    if exclude_set.is_match(path_str.as_ref()) {
        return true;
    }
    for component in path.components() {
        let comp_str = component.as_os_str().to_string_lossy();
        if comp_str.starts_with('.') && comp_str != "." && comp_str != ".." {
            return true;
        }
    }
    false
}

/// Reorders paths so that user-priority directories (Desktop, Documents, Downloads, etc.)
/// appear first. This lets the user search common locations while the rest of the drive
/// is still being indexed.
fn prioritize_paths(paths: Vec<String>) -> Vec<String> {
    const PRIORITY: &[&str] = &[
        "Desktop",
        "Documents",
        "Downloads",
        "Pictures",
        "Music",
        "Videos",
    ];
    let (mut first, rest): (Vec<_>, Vec<_>) = paths.into_iter().partition(|p| {
        std::path::Path::new(p).components().any(|c| {
            c.as_os_str()
                .to_str()
                .is_some_and(|s| PRIORITY.contains(&s))
        })
    });
    first.extend(rest);
    first
}

/// Builds a [`FileEntry`] from a path and its metadata.
///
/// When `mode` is [`IndexMode::MetadataOnly`], `content` is always `None`.
/// When `mode` is [`IndexMode::ContentEnrichment`], text content is extracted for
/// text-extension files.
pub(crate) fn build_file_entry(
    path: &Path,
    metadata: &std::fs::Metadata,
    mode: IndexMode,
) -> FileEntry {
    let file_type = metadata.file_type();
    let kind = if file_type.is_dir() {
        ResultKind::Folder
    } else {
        ResultKind::File
    };

    let size = if file_type.is_file() {
        metadata.len()
    } else {
        0
    };

    #[allow(clippy::cast_possible_wrap)]
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let content = if mode == IndexMode::ContentEnrichment
        && file_type.is_file()
        && is_text_extension(&path.to_string_lossy())
    {
        extract_text_content(path)
    } else {
        None
    };

    FileEntry {
        path: path.to_string_lossy().to_string(),
        name,
        size,
        modified,
        kind,
        content,
    }
}

fn process_entry(entry: &jwalk::DirEntry<((), ())>, mode: IndexMode) -> Result<FileEntry> {
    let path = entry.path();
    let metadata = entry
        .metadata()
        .map_err(|e| QuikFindError::Generic(format!("Metadata: {e}")))?;
    Ok(build_file_entry(&path, &metadata, mode))
}

/// Reads up to 50 KB of text from a file, returning `None` if the file is binary
/// (detected via a fast 512-byte null-byte probe) or too large (>500 KB).
#[must_use]
pub fn extract_text_content(path: &Path) -> Option<String> {
    const MAX_CONTENT_SIZE: u64 = 50 * 1024;
    const MAX_FILE_SIZE: u64 = 500 * 1024;

    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > MAX_FILE_SIZE {
        return None;
    }

    // Fast binary detection: read first 512 bytes and check for null byte
    let mut probe = [0u8; 512];
    let mut file = std::fs::File::open(path).ok()?;
    let n = file.read(&mut probe).ok()?;
    if probe[..n].contains(&0) {
        return None;
    }
    // Reopen — we consumed the probe bytes from the first handle
    drop(file);
    let file = std::fs::File::open(path).ok()?;

    let read_size = metadata.len().min(MAX_CONTENT_SIZE);
    let mut buf = Vec::with_capacity(read_size as usize);
    file.take(read_size).read_to_end(&mut buf).ok()?;

    if buf.contains(&0) {
        return None;
    }

    String::from_utf8(buf).ok()
}
