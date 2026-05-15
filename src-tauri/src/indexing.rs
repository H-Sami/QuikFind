use crate::error::{QuikFindError, Result};
use crate::indexer::Indexer;
use crate::models::{AppSettings, IndexStatus, IndexingProgress};
use crate::search::SearchEngine;
use crate::settings::SettingsDatabase;
use crate::watcher::FileWatcher;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{error, info};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexPhase {
    Idle,
    Metadata,
    Content,
}

impl IndexPhase {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Metadata => "metadata",
            Self::Content => "content",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexRequest {
    Incremental,
    Rebuild,
}

#[derive(Clone)]
pub struct IndexingJob {
    pub paths: Vec<String>,
    pub request: IndexRequest,
    pub settings: AppSettings,
    pub app_handle: AppHandle,
    pub search_engine: Arc<SearchEngine>,
    pub db: Arc<RwLock<SettingsDatabase>>,
    pub watcher: Arc<FileWatcher>,
}

pub struct IndexingSupervisor {
    handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    cancel: Mutex<Option<Arc<AtomicBool>>>,
    status: RwLock<IndexStatus>,
}

impl Default for IndexingSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl IndexingSupervisor {
    #[must_use]
    pub fn new() -> Self {
        Self {
            handle: Mutex::new(None),
            cancel: Mutex::new(None),
            status: RwLock::new(idle_status(0)),
        }
    }

    pub async fn status(&self, indexed_count: u64) -> IndexStatus {
        let mut status = self.status.read().await.clone();
        if status.phase == IndexPhase::Idle.as_str() {
            status.files_indexed = indexed_count;
        }
        status
    }

    pub async fn is_active(&self) -> bool {
        self.handle.lock().await.is_some()
    }

    pub async fn start(self: &Arc<Self>, job: IndexingJob) -> Result<()> {
        if job.request == IndexRequest::Rebuild {
            self.stop(job.search_engine.clone()).await.ok();
        }

        let mut handle_guard = self.handle.lock().await;
        if handle_guard.is_some() {
            return Err(QuikFindError::Generic(
                "Indexing is already in progress".into(),
            ));
        }

        job.watcher.stop()?;

        if job.request == IndexRequest::Rebuild {
            job.search_engine.clear_index().await?;
            let db_guard = job.db.read().await;
            db_guard.clear_indexed_files()?;
        }

        let cancel = Arc::new(AtomicBool::new(false));
        *self.cancel.lock().await = Some(cancel.clone());

        let supervisor = self.clone();
        let task = tokio::spawn(async move {
            let result = run_indexing_lifecycle(supervisor.clone(), job.clone(), cancel).await;

            if let Err(err) = result {
                if !matches!(err, QuikFindError::Cancelled) {
                    error!("Indexing failed: {}", err);
                }
            }

            let indexed_count = {
                let db_guard = job.db.read().await;
                db_guard.get_indexed_file_count().unwrap_or(0)
            };
            supervisor
                .set_status(idle_status(indexed_count), &job.app_handle)
                .await;
            *supervisor.cancel.lock().await = None;
            *supervisor.handle.lock().await = None;
        });

        *handle_guard = Some(task);
        info!("Indexing lifecycle started");
        Ok(())
    }

    pub async fn stop(&self, search_engine: Arc<SearchEngine>) -> Result<()> {
        let handle = {
            if let Some(cancel) = self.cancel.lock().await.as_ref() {
                cancel.store(true, Ordering::Relaxed);
            }
            self.handle.lock().await.take()
        };

        let Some(handle) = handle else {
            return Err(QuikFindError::Generic("No indexing task is running".into()));
        };

        handle
            .await
            .map_err(|e| QuikFindError::Generic(format!("Indexing task join failed: {e}")))?;

        search_engine.commit_reload_invalidate().await.ok();
        *self.cancel.lock().await = None;
        info!("Indexing stopped by user");
        Ok(())
    }

    async fn set_status(&self, status: IndexStatus, app_handle: &AppHandle) {
        *self.status.write().await = status.clone();
        app_handle.emit("index-progress", status).ok();
    }
}

async fn run_indexing_lifecycle(
    supervisor: Arc<IndexingSupervisor>,
    job: IndexingJob,
    cancel: Arc<AtomicBool>,
) -> Result<()> {
    let indexer = Indexer::new(job.search_engine.clone(), job.db.clone());

    supervisor
        .set_status(
            active_status(IndexPhase::Metadata, 0, 0, Vec::new()),
            &job.app_handle,
        )
        .await;

    let metadata_summary = run_progress_phase(
        supervisor.clone(),
        job.app_handle.clone(),
        IndexPhase::Metadata,
        |tx| {
            indexer.index_paths(
                job.paths.clone(),
                &job.settings.excluded_patterns,
                tx,
                cancel.clone(),
            )
        },
    )
    .await?;

    supervisor
        .set_status(
            active_status(
                IndexPhase::Metadata,
                metadata_summary.files_indexed,
                metadata_summary.total_files,
                metadata_summary.errors.clone(),
            ),
            &job.app_handle,
        )
        .await;

    job.app_handle
        .emit(
            "index-phase1-complete",
            active_status(
                IndexPhase::Metadata,
                metadata_summary.files_indexed,
                metadata_summary.total_files,
                metadata_summary.errors,
            ),
        )
        .ok();

    job.watcher.update_watched_paths(
        &job.paths,
        &job.settings.excluded_patterns,
        job.search_engine.clone(),
        job.db.clone(),
    )?;

    supervisor
        .set_status(
            active_status(IndexPhase::Content, 0, 0, Vec::new()),
            &job.app_handle,
        )
        .await;

    run_progress_phase(supervisor, job.app_handle, IndexPhase::Content, |tx| {
        indexer.enrich_content(tx, cancel)
    })
    .await?;

    Ok(())
}

async fn run_progress_phase<F, Fut>(
    supervisor: Arc<IndexingSupervisor>,
    app_handle: AppHandle,
    phase: IndexPhase,
    run: F,
) -> Result<IndexingProgress>
where
    F: FnOnce(mpsc::UnboundedSender<IndexingProgress>) -> Fut,
    Fut: std::future::Future<Output = Result<IndexingProgress>>,
{
    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<IndexingProgress>();
    let progress_handle = tokio::spawn(async move {
        let mut latest = IndexingProgress {
            files_indexed: 0,
            total_files: 0,
            current_file: None,
            errors: Vec::new(),
        };

        while let Some(progress) = progress_rx.recv().await {
            latest = progress.clone();
            supervisor
                .set_status(
                    active_status(
                        phase,
                        progress.files_indexed,
                        progress.total_files,
                        progress.errors,
                    ),
                    &app_handle,
                )
                .await;
        }

        latest
    });

    let result = run(progress_tx.clone()).await;
    drop(progress_tx);
    let latest = progress_handle.await.unwrap_or(IndexingProgress {
        files_indexed: 0,
        total_files: 0,
        current_file: None,
        errors: Vec::new(),
    });

    match result {
        Ok(summary) => Ok(summary),
        Err(QuikFindError::Cancelled) => Err(QuikFindError::Cancelled),
        Err(err) => {
            if latest.errors.is_empty() {
                Err(err)
            } else {
                Err(QuikFindError::Generic(format!(
                    "{err}; {} indexed path errors",
                    latest.errors.len()
                )))
            }
        }
    }
}

fn active_status(
    phase: IndexPhase,
    files_indexed: u64,
    total_files: u64,
    errors: Vec<String>,
) -> IndexStatus {
    let progress_percent = if total_files > 0 {
        #[allow(clippy::cast_precision_loss)]
        {
            (files_indexed as f32 / total_files as f32) * 100.0
        }
    } else {
        0.0
    };

    IndexStatus {
        is_indexing: phase != IndexPhase::Idle,
        phase: phase.as_str().to_string(),
        files_indexed,
        total_files,
        progress_percent,
        last_updated: chrono::Utc::now().timestamp(),
        errors,
    }
}

fn idle_status(files_indexed: u64) -> IndexStatus {
    IndexStatus {
        is_indexing: false,
        phase: IndexPhase::Idle.as_str().to_string(),
        files_indexed,
        total_files: files_indexed,
        progress_percent: 100.0,
        last_updated: chrono::Utc::now().timestamp(),
        errors: Vec::new(),
    }
}
