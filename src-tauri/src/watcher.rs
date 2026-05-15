use crate::error::{QuikFindError, Result};
use crate::indexer::{build_file_entry, build_glob_set, is_excluded, IndexMode};
use crate::models::compute_file_id;
use crate::search::SearchEngine;
use crate::settings::SettingsDatabase;
use globset::GlobSet;
use notify::event::{ModifyKind, RenameMode};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

#[derive(Default)]
struct WatcherChanges {
    upserts: Vec<PathBuf>,
    removes: Vec<PathBuf>,
}

fn spawn_watcher_loop(
    is_running: Arc<AtomicBool>,
    rx: tokio::sync::mpsc::UnboundedReceiver<notify::Result<Event>>,
    search_engine: Arc<SearchEngine>,
    db: Arc<RwLock<SettingsDatabase>>,
    exclude_set: GlobSet,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let debounce_duration = Duration::from_millis(150);
        let cache_size = std::num::NonZeroUsize::new(2000).expect("2000 is non-zero");
        let debounce_map: Arc<parking_lot::Mutex<lru::LruCache<String, Instant>>> =
            Arc::new(parking_lot::Mutex::new(lru::LruCache::new(cache_size)));
        let mut pending_upsert = VecDeque::new();
        let mut pending_remove = VecDeque::new();
        let mut rx = rx;

        let mut flush_interval = tokio::time::interval(Duration::from_millis(300));
        flush_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                Some(Ok(event)) = rx.recv() => {
                    if !is_running.load(Ordering::Relaxed) {
                        break;
                    }

                    let changes = classify_event(&event, &exclude_set);
                    enqueue_deduped(changes.upserts, &mut pending_upsert, &debounce_map, debounce_duration);
                    enqueue_deduped(changes.removes, &mut pending_remove, &debounce_map, debounce_duration);
                }
                _ = flush_interval.tick() => {
                    if pending_upsert.is_empty() && pending_remove.is_empty() {
                        continue;
                    }
                    process_pending(
                        &mut pending_upsert,
                        &mut pending_remove,
                        &search_engine,
                        &db,
                    ).await;
                }
                else => break,
            }
        }

        process_pending(
            &mut pending_upsert,
            &mut pending_remove,
            &search_engine,
            &db,
        )
        .await;

        info!("File watcher loop stopped");
    })
}

fn enqueue_deduped(
    paths: Vec<PathBuf>,
    queue: &mut VecDeque<PathBuf>,
    debounce_map: &parking_lot::Mutex<lru::LruCache<String, Instant>>,
    debounce_duration: Duration,
) {
    let now = Instant::now();
    let mut debounce = debounce_map.lock();
    for path in paths {
        let key = path.to_string_lossy().to_string();
        if let Some(last) = debounce.get(&key) {
            if now.duration_since(*last) < debounce_duration {
                continue;
            }
        }
        debounce.put(key, now);
        queue.push_back(path);
    }
}

fn classify_event(event: &Event, exclude_set: &GlobSet) -> WatcherChanges {
    let paths: Vec<PathBuf> = event
        .paths
        .iter()
        .filter(|path| !is_excluded(path, exclude_set))
        .cloned()
        .collect();

    let mut changes = WatcherChanges::default();
    match &event.kind {
        EventKind::Create(_) => {
            changes
                .upserts
                .extend(paths.into_iter().filter(|path| is_indexable_file(path)));
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if paths.len() >= 2 => {
            changes.removes.push(paths[0].clone());
            if is_indexable_file(&paths[1]) {
                changes.upserts.push(paths[1].clone());
            }
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
            changes.removes.extend(paths);
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
            changes
                .upserts
                .extend(paths.into_iter().filter(|path| is_indexable_file(path)));
        }
        EventKind::Modify(_) => {
            changes
                .upserts
                .extend(paths.into_iter().filter(|path| is_indexable_file(path)));
        }
        EventKind::Remove(_) => {
            changes.removes.extend(paths);
        }
        _ => {}
    }
    changes
}

fn is_indexable_file(path: &Path) -> bool {
    path.exists() && path.is_file()
}

async fn upsert_file(
    path: &Path,
    search_engine: &SearchEngine,
    db: &RwLock<SettingsDatabase>,
) -> Result<bool> {
    if path.is_dir() {
        return Ok(false);
    }

    let Ok(metadata) = std::fs::metadata(path) else {
        return Ok(false);
    };

    let entry = build_file_entry(path, &metadata, IndexMode::ContentEnrichment);
    search_engine.index_document(&entry)?;

    let doc_id = compute_file_id(&entry.path);
    let db_guard = db.read().await;
    db_guard.record_indexed_file(&entry.path, entry.size, entry.modified, &doc_id)?;

    debug!("Indexed file: {}", entry.path);
    Ok(true)
}

async fn handle_remove(
    path: &Path,
    search_engine: &SearchEngine,
    db: &RwLock<SettingsDatabase>,
) -> Result<bool> {
    let path_str = path.to_string_lossy().to_string();
    let db_guard = db.read().await;

    if let Some(doc_id) = db_guard.remove_indexed_file(&path_str)? {
        search_engine.delete_document(&doc_id)?;
        debug!("Removed indexed file: {}", path_str);
        return Ok(true);
    }

    Ok(false)
}

async fn process_pending(
    pending_upsert: &mut VecDeque<PathBuf>,
    pending_remove: &mut VecDeque<PathBuf>,
    search_engine: &SearchEngine,
    db: &RwLock<SettingsDatabase>,
) {
    let mut changed = false;

    while let Some(path) = pending_remove.pop_front() {
        match handle_remove(&path, search_engine, db).await {
            Ok(did_change) => changed |= did_change,
            Err(e) => error!("Watcher remove error for {}: {}", path.display(), e),
        }
    }

    while let Some(path) = pending_upsert.pop_front() {
        match upsert_file(&path, search_engine, db).await {
            Ok(did_change) => changed |= did_change,
            Err(e) => error!("Watcher upsert error for {}: {}", path.display(), e),
        }
    }

    if changed {
        if let Err(e) = search_engine.commit_reload_invalidate().await {
            error!("Watcher commit error: {}", e);
        }
    }
}

pub struct FileWatcher {
    watcher: Arc<std::sync::Mutex<Option<RecommendedWatcher>>>,
    loop_handle: Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
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
            loop_handle: Arc::new(std::sync::Mutex::new(None)),
            is_running: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn start(
        &self,
        paths: &[String],
        excluded_patterns: &[String],
        search_engine: Arc<SearchEngine>,
        db: Arc<RwLock<SettingsDatabase>>,
    ) -> Result<()> {
        if self.is_running.load(Ordering::Relaxed) {
            warn!("File watcher is already running");
            return Ok(());
        }

        let exclude_set = build_glob_set(excluded_patterns)?;
        let (tx, rx) = mpsc::unbounded_channel::<notify::Result<Event>>();
        let mut watcher = RecommendedWatcher::new(
            move |res| {
                tx.send(res).ok();
            },
            Config::default(),
        )
        .map_err(|e| QuikFindError::Generic(format!("Failed to create watcher: {e}")))?;

        for path_str in paths {
            let path = Path::new(path_str);
            if path.exists() {
                watcher.watch(path, RecursiveMode::Recursive).map_err(|e| {
                    QuikFindError::Generic(format!("Failed to watch path '{path_str}': {e}"))
                })?;
                info!("Watching path: {}", path_str);
            } else {
                warn!("Watch path does not exist: {}", path_str);
            }
        }

        *self
            .watcher
            .lock()
            .map_err(|e| QuikFindError::Generic(format!("Watcher lock: {e}")))? = Some(watcher);
        self.is_running.store(true, Ordering::Relaxed);

        let handle =
            spawn_watcher_loop(self.is_running.clone(), rx, search_engine, db, exclude_set);
        *self
            .loop_handle
            .lock()
            .map_err(|e| QuikFindError::Generic(format!("Watcher loop lock: {e}")))? = Some(handle);

        info!("File watcher started for {} paths", paths.len());
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        self.is_running.store(false, Ordering::Relaxed);

        if let Some(watcher) = self
            .watcher
            .lock()
            .map_err(|e| QuikFindError::Generic(format!("Watcher lock: {e}")))?
            .take()
        {
            drop(watcher);
        }

        if let Some(handle) = self
            .loop_handle
            .lock()
            .map_err(|e| QuikFindError::Generic(format!("Watcher loop lock: {e}")))?
            .take()
        {
            handle.abort();
        }

        info!("File watcher stopped");
        Ok(())
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::Relaxed)
    }

    pub fn update_watched_paths(
        &self,
        new_paths: &[String],
        excluded_patterns: &[String],
        search_engine: Arc<SearchEngine>,
        db: Arc<RwLock<SettingsDatabase>>,
    ) -> Result<()> {
        self.stop()?;
        self.start(new_paths, excluded_patterns, search_engine, db)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, RemoveKind};

    fn empty_exclusions() -> GlobSet {
        build_glob_set(&[]).unwrap()
    }

    #[test]
    fn remove_events_preserve_nonexistent_paths() {
        let path = PathBuf::from("C:/definitely/not/here.txt");
        let event = Event::new(EventKind::Remove(RemoveKind::File)).add_path(path.clone());

        let changes = classify_event(&event, &empty_exclusions());

        assert_eq!(changes.removes, vec![path]);
        assert!(changes.upserts.is_empty());
    }

    #[test]
    fn create_events_ignore_directories_and_exclusions() {
        let temp_dir = std::env::temp_dir().join(format!(
            "quikfind-watcher-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let file = temp_dir.join("keep.txt");
        std::fs::write(&file, "hello").unwrap();
        let excluded = temp_dir.join("node_modules").join("skip.txt");
        std::fs::create_dir_all(excluded.parent().unwrap()).unwrap();
        std::fs::write(&excluded, "skip").unwrap();

        let exclusions = build_glob_set(&["**/node_modules/**".to_string()]).unwrap();
        let event = Event::new(EventKind::Create(CreateKind::File))
            .add_path(file.clone())
            .add_path(temp_dir.clone())
            .add_path(excluded);

        let changes = classify_event(&event, &exclusions);

        assert_eq!(changes.upserts, vec![file]);
        assert!(changes.removes.is_empty());

        std::fs::remove_dir_all(temp_dir).unwrap();
    }

    #[test]
    fn rename_both_removes_old_and_upserts_new() {
        let temp_dir = std::env::temp_dir().join(format!(
            "quikfind-rename-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let old = temp_dir.join("old.txt");
        let new = temp_dir.join("new.txt");
        std::fs::write(&new, "hello").unwrap();
        let event = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
            .add_path(old.clone())
            .add_path(new.clone());

        let changes = classify_event(&event, &empty_exclusions());

        assert_eq!(changes.removes, vec![old]);
        assert_eq!(changes.upserts, vec![new]);

        std::fs::remove_dir_all(temp_dir).unwrap();
    }
}
