use crate::error::{QuikFindError, Result};
use crate::models::{compute_file_id, is_text_extension, FileEntry, ResultKind};
use crate::search::SearchEngine;
use crate::settings::SettingsDatabase;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

fn spawn_watcher_loop(
    is_running: Arc<AtomicBool>,
    rx: tokio::sync::mpsc::UnboundedReceiver<notify::Result<Event>>,
    search_engine: Arc<SearchEngine>,
    db: Arc<RwLock<SettingsDatabase>>,
) {
    tokio::spawn(async move {
        let debounce_duration = Duration::from_millis(150);
        let debounce_map: Arc<parking_lot::Mutex<
            lru::LruCache<String, Instant>,
        >> = Arc::new(parking_lot::Mutex::new(
            // SAFETY: 2000 is a non-zero compile-time constant
            #[allow(clippy::unwrap_used)]
            lru::LruCache::new(std::num::NonZeroUsize::new(2000).unwrap()),
        ));
        let mut pending_create: std::collections::VecDeque<std::path::PathBuf> = std::collections::VecDeque::new();
        let mut pending_modify: std::collections::VecDeque<std::path::PathBuf> = std::collections::VecDeque::new();
        let mut pending_remove: std::collections::VecDeque<std::path::PathBuf> = std::collections::VecDeque::new();
        let needs_commit = std::sync::atomic::AtomicBool::new(false);
        let needs_commit = &needs_commit;
        let mut rx = rx;

        let mut flush_interval = tokio::time::interval(Duration::from_millis(300));
        flush_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                Some(Ok(event)) = rx.recv() => {
                    if !is_running.load(Ordering::Relaxed) {
                        break;
                    }
                    let paths: Vec<_> = event.paths.iter()
                        .filter(|p| p.exists())
                        .cloned()
                        .collect();
                    if paths.is_empty() { continue; }

                    let now = Instant::now();
                    let deduped: Vec<_> = {
                        let mut debounce = debounce_map.lock();
                        paths.into_iter().filter(|p| {
                            let key = p.to_string_lossy().to_string();
                            if let Some(last) = debounce.get(&key) {
                                if now.duration_since(*last) < debounce_duration { return false; }
                            }
                            debounce.put(key, now);
                            true
                        }).collect()
                    };
                    if deduped.is_empty() { continue; }
                    match event.kind {
                        EventKind::Create(_) => {
                            for p in &deduped { if !p.is_dir() { pending_create.push_back(p.clone()); } }
                        }
                        EventKind::Modify(_) => {
                            for p in &deduped { if !p.is_dir() { pending_modify.push_back(p.clone()); } }
                        }
                        EventKind::Remove(_) => {
                            for p in &deduped { pending_remove.push_back(p.clone()); }
                        }
                        _ => {}
                    }
                }
                _ = flush_interval.tick() => {
                    if pending_create.is_empty() && pending_modify.is_empty() && pending_remove.is_empty() {
                        continue;
                    }
                    process_pending(
                        &mut pending_create,
                        &mut pending_modify,
                        &mut pending_remove,
                        needs_commit,
                        &search_engine,
                        &db,
                    ).await;
                }
            }

            if !is_running.load(Ordering::Relaxed)
                && pending_create.is_empty()
                && pending_modify.is_empty()
                && pending_remove.is_empty()
            {
                break;
            }
        }

        process_pending(
            &mut pending_create,
            &mut pending_modify,
            &mut pending_remove,
            needs_commit,
            &search_engine,
            &db,
        ).await;

        info!("File watcher stopped");
    });
}

async fn process_pending(
    pending_create: &mut std::collections::VecDeque<std::path::PathBuf>,
    pending_modify: &mut std::collections::VecDeque<std::path::PathBuf>,
    pending_remove: &mut std::collections::VecDeque<std::path::PathBuf>,
    needs_commit: &std::sync::atomic::AtomicBool,
    search_engine: &SearchEngine,
    db: &RwLock<SettingsDatabase>,
) {
    while let Some(path) = pending_remove.pop_front() {
        if let Err(e) = handle_remove(&path, search_engine, db).await {
            error!("Watcher remove error for {}: {}", path.display(), e);
        }
        needs_commit.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    while let Some(path) = pending_modify.pop_front() {
        if let Err(e) = handle_modify(&path, search_engine, db).await {
            error!("Watcher modify error for {}: {}", path.display(), e);
        }
        needs_commit.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    while let Some(path) = pending_create.pop_front() {
        if let Err(e) = handle_create(&path, search_engine, db).await {
            error!("Watcher create error for {}: {}", path.display(), e);
        }
    }
    if needs_commit.load(std::sync::atomic::Ordering::Relaxed) {
        search_engine.commit().ok();
        search_engine.reload_searcher().await.ok();
        needs_commit.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

pub struct FileWatcher {
    watcher: Arc<std::sync::Mutex<Option<RecommendedWatcher>>>,
    is_running: Arc<AtomicBool>,
}

impl Default for FileWatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl FileWatcher {
    #[must_use]
    pub fn new() -> Self {
        Self {
            watcher: Arc::new(std::sync::Mutex::new(None)),
            is_running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Starts watching the given paths for file changes.
    ///
    /// # Errors
    /// Returns an error if the watcher cannot be created or paths cannot be watched.
    ///
    /// # Panics
    /// Panics if the LRU cache size is not a valid non-zero value.
    pub fn start(
        &self,
        paths: &[String],
        search_engine: Arc<SearchEngine>,
        db: Arc<RwLock<SettingsDatabase>>,
    ) -> Result<()> {
        if self.is_running.load(Ordering::Relaxed) {
            warn!("File watcher is already running");
            return Ok(());
        }

        let (tx, rx) = mpsc::unbounded_channel::<notify::Result<Event>>();

    let mut watcher = RecommendedWatcher::new(
        move |res| {
            tx.send(res).ok();
        },
        Config::default(),
    )
    .map_err(|e| {
        QuikFindError::Generic(format!("Failed to create watcher: {e}"))
    })?;

        for path_str in paths {
            let path = Path::new(path_str);
            if path.exists() {
                watcher
                    .watch(path, RecursiveMode::Recursive)
                    .map_err(|e| {
                        QuikFindError::Generic(format!(
                            "Failed to watch path '{path_str}': {e}"
                        ))
                    })?;
                info!("Watching path: {}", path_str);
            } else {
                warn!("Watch path does not exist: {}", path_str);
            }
        }

        let mut w = self.watcher.lock().map_err(|e| {
            QuikFindError::Generic(format!("Watcher lock: {e}"))
        })?;
        *w = Some(watcher);
        self.is_running.store(true, Ordering::Relaxed);

        spawn_watcher_loop(
            self.is_running.clone(),
            rx,
            search_engine,
            db,
        );

        info!("File watcher started for {} paths", paths.len());
        Ok(())
    }

    /// Stops the file watcher.
    ///
    /// # Errors
    /// Returns an error if the watcher lock is poisoned.
    pub fn stop(&self) -> Result<()> {
        self.is_running.store(false, Ordering::Relaxed);

        let mut w = self.watcher.lock().map_err(|e| {
            QuikFindError::Generic(format!("Watcher lock: {e}"))
        })?;

        if let Some(watcher) = w.take() {
            drop(watcher);
            info!("File watcher stopped");
        }

        Ok(())
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::Relaxed)
    }

    /// Updates the set of watched paths.
    ///
    /// # Errors
    /// Returns an error if the watcher cannot be restarted.
    pub fn update_watched_paths(
        &self,
        new_paths: &[String],
        search_engine: Arc<SearchEngine>,
        db: Arc<RwLock<SettingsDatabase>>,
    ) -> Result<()> {
        self.stop()?;
        std::thread::sleep(Duration::from_millis(50));
        self.start(new_paths, search_engine, db)
    }
}

async fn handle_create(
    path: &Path,
    search_engine: &SearchEngine,
    db: &RwLock<SettingsDatabase>,
) -> Result<()> {
    if path.is_dir() {
        return Ok(());
    }

    let Ok(metadata) = std::fs::metadata(path) else {
        return Ok(());
    };

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let size = metadata.len();
    // REASON: Unix timestamps fit in i64 for billions of years; safe truncation
    #[allow(clippy::cast_possible_wrap)]
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);

    let content = if metadata.is_file() && is_text_extension(&path.to_string_lossy()) {
        crate::indexer::extract_text_content(path)
    } else {
        None
    };

    let entry = FileEntry {
        path: path.to_string_lossy().to_string(),
        name,
        size,
        modified,
        kind: ResultKind::File,
        content,
    };

    search_engine.index_document(&entry)?;

    let doc_id = compute_file_id(&entry.path, entry.size, entry.modified);
    let db = db.read().await;
    db.record_indexed_file(&entry.path, entry.size, entry.modified, &doc_id)
        .ok();

    debug!("Indexed new file: {}", entry.path);
    Ok(())
}

async fn handle_modify(
    path: &Path,
    search_engine: &SearchEngine,
    db: &RwLock<SettingsDatabase>,
) -> Result<()> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return Ok(());
    };

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let size = metadata.len();
    // REASON: Unix timestamps fit in i64 for billions of years; safe truncation
    #[allow(clippy::cast_possible_wrap)]
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);

    let db_guard = db.read().await;
    if let Ok(Some(old_doc_id)) = db_guard.remove_indexed_file(&path.to_string_lossy()) {
        search_engine.delete_document(&old_doc_id).ok();
    }

    let content = if metadata.is_file() && is_text_extension(&path.to_string_lossy()) {
        crate::indexer::extract_text_content(path)
    } else {
        None
    };

    let entry = FileEntry {
        path: path.to_string_lossy().to_string(),
        name,
        size,
        modified,
        kind: ResultKind::File,
        content,
    };

    search_engine.index_document(&entry)?;

    let new_doc_id = compute_file_id(&entry.path, entry.size, entry.modified);
    db_guard
        .record_indexed_file(&entry.path, entry.size, entry.modified, &new_doc_id)
        .ok();

    debug!("Re-indexed modified file: {}", entry.path);
    Ok(())
}

async fn handle_remove(
    path: &Path,
    search_engine: &SearchEngine,
    db: &RwLock<SettingsDatabase>,
) -> Result<()> {
    let path_str = path.to_string_lossy().to_string();
    let db_guard = db.read().await;

    if let Ok(Some(doc_id)) = db_guard.remove_indexed_file(&path_str) {
        search_engine.delete_document(&doc_id).ok();
        debug!("Removed indexed file: {}", path_str);
    }

    Ok(())
}
