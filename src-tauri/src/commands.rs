// REASON: Tauri command signatures are fixed; doc comments would be purely cosmetic
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use crate::models::{
    AppResult, AppSettings, HistoryItem, IndexStatus, SearchResults,
};
use crate::plugins::PluginRegistry;
use crate::search::SearchEngine;
use crate::settings::SettingsDatabase;
use crate::watcher::FileWatcher;
use parking_lot::Mutex as ParkingMutex;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tauri_plugin_global_shortcut::Shortcut;
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info};

pub struct AppState {
    pub search_engine: Arc<SearchEngine>,
    pub db: Arc<RwLock<SettingsDatabase>>,
    pub settings: Arc<RwLock<AppSettings>>,
    pub watcher: Arc<FileWatcher>,
    pub app_scanner: Arc<crate::apps::AppScanner>,
    pub plugin_registry: Arc<Mutex<PluginRegistry>>,
    pub indexing_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pub is_indexing: Arc<std::sync::atomic::AtomicBool>,
    pub active_hotkey: Arc<ParkingMutex<Option<Shortcut>>>,
}

#[tauri::command]
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
pub async fn open_path(
    path: String,
    app: Option<String>,
    _app_handle: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if let Some(app_name) = app {
        let result = if cfg!(target_os = "windows") {
            std::process::Command::new(&app_name).arg(&path).spawn()
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
pub async fn start_indexing(
    paths: Vec<String>,
    app_handle: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let actual_paths: Vec<String> = if paths.is_empty() {
        #[cfg(target_os = "windows")]
        {
            crate::platform::get_all_windows_drives()
        }
        #[cfg(not(target_os = "windows"))]
        {
            vec!["/".to_string()]
        }
    } else {
        paths
    };

    let result = crate::run_indexing(actual_paths, &state, &app_handle).await;
    if result.is_ok() {
        info!("Indexing started");
    } else if let Err(e) = &result {
        error!("Failed to start indexing: {}", e);
    }
    result.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn stop_indexing(state: tauri::State<'_, AppState>) -> Result<(), String> {
    {
        let mut handle = state.indexing_handle.lock().await;
        handle.take().map_or_else(
            || Err("No indexing task is running".to_string()),
            |h| {
                h.abort();
                state.is_indexing.store(false, std::sync::atomic::Ordering::Relaxed);
                info!("Indexing aborted by user");
                Ok(())
            },
        )?;
    }
    state
        .search_engine
        .finish_batch_index()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
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
pub async fn get_settings(
    state: tauri::State<'_, AppState>,
) -> Result<AppSettings, String> {
    let db = state.db.read().await;
    db.load_settings().map_err(|e| e.to_string())
}

#[tauri::command]
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

    crate::hotkey::register_hotkey(&app_handle, &settings.hotkey, &state);

    info!("Settings updated");
    app_handle.emit("settings-updated", settings).ok();

    Ok(())
}

#[tauri::command]
pub async fn search_apps(
    query: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AppResult>, String> {
    state.app_scanner.search_apps(&query).await.map_err(|e| e.to_string())
}

#[tauri::command]
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
pub async fn get_history(
    limit: u32,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<HistoryItem>, String> {
    let db = state.db.read().await;
    db.get_history(limit).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_to_history(
    item: HistoryItem,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.read().await;
    db.add_history(&item).map_err(|e| e.to_string())
}

#[tauri::command]
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
pub async fn scan_apps_now(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AppResult>, String> {
    state.app_scanner.scan_and_cache_apps().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_autostart(
    enabled: bool,
    app_handle: AppHandle,
) -> Result<(), String> {
    crate::set_autostart(enabled, &app_handle)
}

#[tauri::command]
pub async fn get_window_state(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    let db = state.db.read().await;
    db.get_window_state().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_window_state(
    json: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.read().await;
    db.save_window_state(&json).map_err(|e| e.to_string())
}

#[tauri::command]
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
