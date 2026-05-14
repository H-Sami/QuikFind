use crate::apps::AppScanner;
use crate::models::{
    AppResult, AppSettings, HistoryItem, IndexStatus, SearchResults,
};
use crate::plugins::PluginRegistry;
use crate::search::SearchEngine;
use crate::settings::SettingsDatabase;
use crate::watcher::FileWatcher;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{error, info};

pub struct AppState {
    pub search_engine: Arc<SearchEngine>,
    pub db: Arc<RwLock<SettingsDatabase>>,
    pub settings: Arc<RwLock<AppSettings>>,
    pub watcher: Arc<FileWatcher>,
    pub app_scanner: Arc<AppScanner>,
    pub plugin_registry: Arc<Mutex<PluginRegistry>>,
    pub indexing_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pub is_indexing: Arc<std::sync::atomic::AtomicBool>,
}

#[tauri::command]
/// Searches indexed files and folders.
///
/// # Errors
/// Returns an error if the search engine is not available.
pub async fn search(
    query: String,
    limit: u32,
    offset: u32,
    state: tauri::State<'_, AppState>,
) -> Result<SearchResults, String> {
    let settings = {
        let db = state.db.read().await;
        db.load_settings().map_err(|e| e.to_string())?
    };

    let results = state
        .search_engine
        .perform_search(
            &query,
            limit,
            offset,
            settings.fuzzy_threshold,
            settings.max_results,
            settings.enable_content_search,
        )
        .await
        .map_err(|e| e.to_string())?;

    Ok(results)
}

#[tauri::command]
/// Opens a file or directory, optionally with a specific app.
///
/// # Errors
/// Returns an error if the path cannot be opened.
pub async fn open_path(
    path: String,
    app: Option<String>,
    _app_handle: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if let Some(app_name) = app {
        let result = if cfg!(target_os = "windows") {
            std::process::Command::new("cmd")
                .args(["/C", "start", "", &path])
                .spawn()
        } else if cfg!(target_os = "macos") {
            std::process::Command::new("open")
                .arg("-a")
                .arg(&app_name)
                .arg(&path)
                .spawn()
        } else {
            std::process::Command::new(&app_name)
                .arg(&path)
                .spawn()
        };

        result.map_err(|e| format!("Failed to open with app '{app_name}': {e}"))?;
    } else {
        open::that(&path).map_err(|e| format!("Failed to open '{path}': {e}"))?;
    }

    let p = std::path::Path::new(&path);
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let history_item = HistoryItem {
        id: path.clone(),
        path: path.clone(),
        name,
        kind: if p.is_dir() {
            "Folder".to_string()
        } else {
            "File".to_string()
        },
        opened_at: chrono::Utc::now().timestamp(),
    };

    let db = state.db.read().await;
    db.add_history(&history_item).ok();

    Ok(())
}

#[tauri::command]
/// Starts indexing files at the given paths.
/// If no paths are provided and none are configured, indexing is prevented
/// and all previous index data is cleared.
///
/// # Errors
/// Returns an error if indexing is already in progress.
pub async fn start_indexing(
    paths: Vec<String>,
    app_handle: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if state.is_indexing.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("Indexing is already in progress".to_string());
    }

    let settings = {
        let db_guard = state.db.read().await;
        db_guard.load_settings().map_err(|e| e.to_string())?
    };

    let actual_paths: Vec<String> = if paths.is_empty() {
        #[cfg(target_os = "windows")]
        {
            get_all_windows_drives()
        }
        #[cfg(not(target_os = "windows"))]
        {
            vec!["/".to_string()] // Fallback for other OS
        }
    } else {
        paths
    };

    // === Normal indexing flow ===
    state.is_indexing.store(true, std::sync::atomic::Ordering::Relaxed);

    let search_engine = state.search_engine.clone();
    let db = state.db.clone();
    let watcher = state.watcher.clone();
    let is_indexing = state.is_indexing.clone();
    let indexing_handle = state.indexing_handle.clone();

    let excluded = settings.excluded_patterns.clone();
    let paths_for_spawn = actual_paths.clone();
    let path_count = actual_paths.len();
    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();

    let handle = tokio::spawn(async move {
        let indexer = crate::indexer::Indexer::new(search_engine.clone(), db.clone());

        if let Err(e) = indexer
            .index_paths(paths_for_spawn.clone(), &excluded, progress_tx)
            .await
        {
            error!("Indexing failed: {}", e);
        }

        if let Err(e) = watcher.update_watched_paths(
            &paths_for_spawn,
            search_engine.clone(),
            db.clone(),
        ) {
            error!("Failed to update watched paths: {}", e);
        }

        is_indexing.store(false, std::sync::atomic::Ordering::Relaxed);
    });

    let app_handle_clone = app_handle.clone();
    tokio::spawn(async move {
        while let Some(progress) = progress_rx.recv().await {
            #[allow(clippy::cast_precision_loss)]
            let progress_percent = if progress.total_files > 0 {
                (progress.files_indexed as f32 / progress.total_files as f32) * 100.0
            } else {
                0.0
            };

            let status = IndexStatus {
                is_indexing: true,
                files_indexed: progress.files_indexed,
                total_files: progress.total_files,
                progress_percent,
                last_updated: chrono::Utc::now().timestamp(),
                errors: progress.errors,
            };
            app_handle_clone.emit("index-progress", status).ok();
        }

        app_handle_clone
            .emit(
                "index-progress",
                IndexStatus {
                    is_indexing: false,
                    files_indexed: 0,
                    total_files: 0,
                    progress_percent: 100.0,
                    last_updated: chrono::Utc::now().timestamp(),
                    errors: Vec::new(),
                },
            )
            .ok();
    });

    let mut h = indexing_handle.lock().await;
    *h = Some(handle);

    info!("Indexing started for {} paths", path_count);
    Ok(())
}

#[tauri::command]
/// Stops the current indexing task.
///
/// # Errors
/// Returns an error if no indexing task is running.
pub async fn stop_indexing(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mut handle = state.indexing_handle.lock().await;
    handle.take().map_or_else(
        || Err("No indexing task is running".to_string()),
        |h| {
            h.abort();
            state
                .is_indexing
                .store(false, std::sync::atomic::Ordering::Relaxed);
            info!("Indexing stopped by user");
            Ok(())
        },
    )
}

#[tauri::command]
/// Returns the current indexing status.
///
/// # Errors
/// Returns an error if the database cannot be read.
pub async fn get_index_status(
    state: tauri::State<'_, AppState>,
) -> Result<IndexStatus, String> {
    let db = state.db.read().await;
    let files_indexed = db.get_indexed_file_count().map_err(|e| e.to_string())?;

    Ok(IndexStatus {
        is_indexing: state.is_indexing.load(std::sync::atomic::Ordering::Relaxed),
        files_indexed,
        total_files: 0,
        progress_percent: 0.0,
        last_updated: chrono::Utc::now().timestamp(),
        errors: Vec::new(),
    })
}

#[tauri::command]
/// Gets the current application settings.
///
/// # Errors
/// Returns an error if the database cannot be read.
pub async fn get_settings(
    state: tauri::State<'_, AppState>,
) -> Result<AppSettings, String> {
    let db = state.db.read().await;
    db.load_settings().map_err(|e| e.to_string())
}

#[tauri::command]
/// Updates the application settings.
///
/// # Errors
/// Returns an error if the database cannot be written.
pub async fn update_settings(
    settings: AppSettings,
    app_handle: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    {
        let db = state.db.read().await;
        db.save_settings(&settings).map_err(|e| e.to_string())?;
    }

    let mut app_settings = state.settings.write().await;
    *app_settings = settings.clone();

    crate::register_hotkey(&app_handle, &settings.hotkey);

    info!("Settings updated");
    app_handle.emit("settings-updated", settings).ok();

    Ok(())
}

#[tauri::command]
/// Searches cached applications by query.
///
/// # Errors
/// Returns an error if the database cannot be read.
pub async fn search_apps(
    query: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AppResult>, String> {
    state.app_scanner.search_apps(&query).await.map_err(|e| e.to_string())
}

#[tauri::command]
/// Launches an application by its ID.
///
/// # Errors
/// Returns an error if the app is not found or cannot be launched.
pub async fn launch_app(
    app_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.read().await;
    let apps = db.get_cached_apps().map_err(|e| e.to_string())?;

    let app = apps.iter().find(|(id, _, _)| id == &app_id);
    match app {
        Some((_, _, path)) => crate::apps::launch_app(path),
        None => Err(format!("App with id '{app_id}' not found")),
    }
}

#[tauri::command]
/// Gets the history of opened files.
///
/// # Errors
/// Returns an error if the database cannot be read.
pub async fn get_history(
    limit: u32,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<HistoryItem>, String> {
    let db = state.db.read().await;
    db.get_history(limit).map_err(|e| e.to_string())
}

#[tauri::command]
/// Adds an item to the history.
///
/// # Errors
/// Returns an error if the database cannot be written.
pub async fn add_to_history(
    item: HistoryItem,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.read().await;
    db.add_history(&item).map_err(|e| e.to_string())
}

#[tauri::command]
/// Clears the index and prepares for reindexing.
///
/// # Errors
/// Returns an error if the index cannot be cleared.
pub async fn reindex_all(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state.search_engine.clear_index().await.map_err(|e| e.to_string())?;

    let db = state.db.read().await;
    db.clear_indexed_files().map_err(|e| e.to_string())?;

    info!("Index cleared, ready for reindexing");
    Ok(())
}

#[tauri::command]
/// Scans for installed applications now.
///
/// # Errors
/// Returns an error if scanning fails.
pub async fn scan_apps_now(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AppResult>, String> {
    state.app_scanner.scan_and_cache_apps().await.map_err(|e| e.to_string())
}

#[tauri::command]
/// Sets whether `QuikFind` should launch on Windows startup.
///
/// # Errors
/// Returns an error if the autostart configuration fails.
pub async fn set_autostart(
    enabled: bool,
    app_handle: AppHandle,
) -> Result<(), String> {
    crate::set_autostart(enabled, &app_handle)
}

#[tauri::command]
/// Gets the saved window state.
///
/// # Errors
/// Returns an error if the database cannot be read.
pub async fn get_window_state(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    let db = state.db.read().await;
    db.get_window_state().map_err(|e| e.to_string())
}

#[tauri::command]
/// Saves the window state.
///
/// # Errors
/// Returns an error if the database cannot be written.
pub async fn save_window_state(
    json: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.read().await;
    db.save_window_state(&json).map_err(|e| e.to_string())
}

#[tauri::command]
/// Gets the list of registered plugins.
///
/// # Errors
/// Returns an error if the plugin registry cannot be accessed.
pub async fn get_plugins(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let registry = state.plugin_registry.lock().await;
    let plugins = registry.list_plugins();
    Ok(plugins
        .into_iter()
        .map(|(name, version)| {
            serde_json::json!({
                "name": name,
                "version": version,
            })
        })
        .collect())
}

#[cfg(target_os = "windows")]
pub(crate) fn get_all_windows_drives() -> Vec<String> {
    let mut drives = Vec::new();

    for letter in b'A'..=b'Z' {
        let drive = format!("{}:\\", letter as char);
        if std::fs::metadata(&drive).is_ok() {
            drives.push(format!("{drive}\\**"));
        }
    }

    if drives.is_empty() {
        drives.push("C:\\**".to_string());
    }

    drives
}
