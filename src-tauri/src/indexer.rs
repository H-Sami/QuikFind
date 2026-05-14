use crate::error::{QuikFindError, Result};
use crate::models::{compute_file_id, is_text_extension, FileEntry, ResultKind};
use crate::search::SearchEngine;
use crate::settings::SettingsDatabase;
use globset::{Glob, GlobSet, GlobSetBuilder};
use jwalk::WalkDir;
use rayon::prelude::*;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

const COMMIT_BATCH_SIZE: usize = 4_000;
const MAX_BINARY_SKIP_SIZE: u64 = 512 * 1_024;
const PROGRESS_INTERVAL_MS: u64 = 500;

struct IndexingContext<'a> {
    exclude_set: &'a GlobSet,
    errors: &'a parking_lot::Mutex<Vec<String>>,
    files_indexed: &'a AtomicU64,
    total_files: &'a AtomicU64,
    last_progress: &'a parking_lot::Mutex<Instant>,
    progress_tx: &'a mpsc::UnboundedSender<crate::models::IndexingProgress>,
}

pub struct Indexer {
    search_engine: Arc<SearchEngine>,
    db: Arc<RwLock<SettingsDatabase>>,
}

impl Indexer {
    pub const fn new(search_engine: Arc<SearchEngine>, db: Arc<RwLock<SettingsDatabase>>) -> Self {
        Self { search_engine, db }
    }

    /// Indexes all files in the given paths.
    ///
    /// # Errors
    /// Returns an error if indexing fails.
    ///
    /// # Panics
    /// Panics if a mutex is poisoned.
    pub async fn index_paths(
        &self,
        paths: Vec<String>,
        excluded_patterns: &[String],
        progress_tx: mpsc::UnboundedSender<crate::models::IndexingProgress>,
    ) -> Result<()> {
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
        };

        self.search_engine.start_batch_index()?;

        progress_tx
            .send(crate::models::IndexingProgress {
                files_indexed: 0,
                total_files: 0,
                current_file: None,
                errors: Vec::new(),
            })
            .ok();

        for path_str in &paths {
            let path = Path::new(path_str);
            if !path.exists() {
                warn!("Index path does not exist: {}", path_str);
                continue;
            }

            if path.is_file() {
                if let Err(e) = self.index_file(path, ctx.exclude_set, ctx.errors, ctx.files_indexed) {
                    error!("Failed to index file {}: {}", path_str, e);
                }
                total_files.fetch_add(1, Ordering::Relaxed);
                continue;
            }

            let entries: Vec<_> = WalkDir::new(path)
                .skip_hidden(false)
                .follow_links(false)
                .into_iter()
                .filter_map(std::result::Result::ok)
                .collect();

            total_files.fetch_add(entries.len() as u64, Ordering::Relaxed);

            self.process_entries_chunks(&entries, &ctx).await;
        }

        self.search_engine.finish_batch_index().await?;

        let elapsed = start.elapsed();
        let indexed_count = files_indexed.load(Ordering::Relaxed);
        // REASON: performance metric for logging; precision loss is acceptable
        #[allow(clippy::cast_precision_loss)]
        let files_per_min = indexed_count as f64 / elapsed.as_secs_f64() * 60.0;
        info!(
            "Indexing complete: {indexed_count} files in {elapsed:?} ({files_per_min:.0} files/min)",
        );

        progress_tx
            .send(crate::models::IndexingProgress {
                files_indexed: indexed_count,
                total_files: total_files.load(Ordering::Relaxed),
                current_file: None,
                errors: errors.lock().clone(),
            })
            .ok();

        Ok(())
    }

    async fn process_entries_chunks(
        &self,
        entries: &[jwalk::DirEntry<((), ())>],
        ctx: &IndexingContext<'_>,
    ) {
        for chunk in entries.chunks(COMMIT_BATCH_SIZE) {
            let results: Vec<_> = chunk
                .par_iter()
                .filter_map(|entry| {
                    let entry_path = entry.path();
                    if is_excluded(&entry_path, ctx.exclude_set) {
                        return None;
                    }
                    let Ok(metadata) = entry.metadata() else {
                        return None;
                    };
                    if metadata.is_file()
                        && metadata.len() > MAX_BINARY_SKIP_SIZE
                        && !is_text_extension(&entry_path.to_string_lossy())
                    {
                        return None;
                    }
                    match process_entry(entry) {
                        Ok(file_entry) => Some(file_entry),
                        Err(e) => {
                        // SAFETY: error mutex only held briefly, never across await
                        ctx.errors.lock().push(format!("{}: {}", entry_path.display(), e));
                        None
                        }
                    }
                })
                .collect();

            let batch_count = results.len();
            for file_entry in &results {
                let db = self.db.read().await;
                let doc_id = compute_file_id(
                    &file_entry.path,
                    file_entry.size,
                    file_entry.modified,
                );
                match self.search_engine.index_document(file_entry) {
                    Ok(()) => {
                        db.record_indexed_file(
                            &file_entry.path,
                            file_entry.size,
                            file_entry.modified,
                            &doc_id,
                        )
                        .ok();
                        ctx.files_indexed.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        ctx.errors.lock().push(format!("{}: {}", file_entry.path, e));
                    }
                }
            }

            let indexed_count = ctx.files_indexed.load(Ordering::Relaxed);
            if indexed_count % (COMMIT_BATCH_SIZE as u64) < batch_count as u64 {
                self.search_engine.commit().ok();
            }

            {
                // SAFETY: progress mutex never held across await points
                let mut last = ctx.last_progress.lock();
                if last.elapsed() >= std::time::Duration::from_millis(PROGRESS_INTERVAL_MS) {
                    ctx.progress_tx
                        .send(crate::models::IndexingProgress {
                            files_indexed: indexed_count,
                            total_files: ctx.total_files.load(Ordering::Relaxed),
                            current_file: results.last().map(|r| r.path.clone()),
                            errors: ctx.errors.lock().clone(),
                        })
                        .ok();
                    *last = Instant::now();
                }
            }
        }
    }

    fn index_file(
        &self,
        path: &Path,
        exclude_set: &GlobSet,
        errors: &parking_lot::Mutex<Vec<String>>,
        files_indexed: &AtomicU64,
    ) -> Result<()> {
        if is_excluded(path, exclude_set) {
            return Ok(());
        }

        match process_single_file(path) {
            Ok(file_entry) => {
                self.search_engine
                    .index_document(&file_entry)
                    .map_err(|e| QuikFindError::Generic(format!("Index doc: {e}")))?;
                files_indexed.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(e) => {
                errors.lock().push(format!("{}: {}", path.display(), e));
                Err(e)
            }
        }
    }
}

fn build_glob_set(patterns: &[String]) -> Result<GlobSet> {
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

fn is_excluded(path: &Path, exclude_set: &GlobSet) -> bool {
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

fn process_entry(entry: &jwalk::DirEntry<((), ())>) -> Result<FileEntry> {
    let path = entry.path();
    let metadata = entry
        .metadata()
        .map_err(|e| QuikFindError::Generic(format!("Metadata: {e}")))?;

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

    // REASON: Unix timestamps fit in i64 for billions of years; safe truncation
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

    let content = if file_type.is_file() && is_text_extension(&path.to_string_lossy()) {
        extract_text_content(&path)
    } else {
        None
    };

    Ok(FileEntry {
        path: path.to_string_lossy().to_string(),
        name,
        size,
        modified,
        kind,
        content,
    })
}

fn process_single_file(path: &Path) -> Result<FileEntry> {
    let metadata = std::fs::metadata(path).map_err(QuikFindError::Io)?;

    let size = metadata.len();
    // REASON: Unix timestamps fit in i64 for billions of years; safe truncation
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

    let content = if metadata.is_file() && is_text_extension(&path.to_string_lossy()) {
        extract_text_content(path)
    } else {
        None
    };

    Ok(FileEntry {
        path: path.to_string_lossy().to_string(),
        name,
        size,
        modified,
        kind: ResultKind::File,
        content,
    })
}

#[must_use]
pub fn extract_text_content(path: &Path) -> Option<String> {
    const MAX_CONTENT_SIZE: u64 = 50 * 1024;

    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > MAX_CONTENT_SIZE * 10 {
        return None;
    }

    let read_size = metadata.len().min(MAX_CONTENT_SIZE);
    let file = std::fs::File::open(path).ok()?;

    let mut buf = Vec::with_capacity(read_size as usize);
    file.take(read_size).read_to_end(&mut buf).ok()?;

    if buf.contains(&0) {
        return None;
    }

    String::from_utf8(buf).ok()
}
