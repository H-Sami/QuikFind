// REASON: Tauri command signatures are fixed; doc comments would be purely cosmetic.
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use crate::desktop_listener::DesktopListener;
use crate::indexing::{IndexRequest, IndexingJob, IndexingSupervisor};
use crate::models::{
    AppResult, AppSettings, HistoryItem, IndexStatus, ResultKind, SearchResult, SearchResults,
};
use crate::plugins::PluginRegistry;
use crate::search::SearchEngine;
use crate::settings::SettingsDatabase;
use crate::watcher::FileWatcher;
use parking_lot::Mutex as ParkingMutex;
use std::sync::Arc;
use std::time::Instant;
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
    pub indexing: Arc<IndexingSupervisor>,
    pub desktop_listener: Arc<DesktopListener>,
    pub active_hotkey: Arc<ParkingMutex<Option<Shortcut>>>,
}

#[tauri::command]
pub async fn search(
    query: String,
    limit: u32,
    offset: u32,
    state: tauri::State<'_, AppState>,
) -> Result<SearchResults, String> {
    let start = Instant::now();
    let settings = state.settings.read().await.clone();
    let effective_limit = limit.min(settings.max_results);
    let use_cache = !state.indexing.is_active().await;

    let mut file_results = state
        .search_engine
        .perform_search(
            &query,
            effective_limit,
            offset,
            settings.max_results,
            settings.enable_content_search,
            use_cache,
        )
        .await
        .map_err(|e| e.to_string())?;

    let app_results = if query.trim().is_empty() {
        state.app_scanner.cached_apps().await
    } else {
        state.app_scanner.search_apps(&query).await
    }
    .map_err(|e| e.to_string())?;

    let merged = merge_results(
        file_results.results,
        app_results,
        query.trim().is_empty(),
        effective_limit,
        offset,
    );

    #[allow(clippy::cast_possible_truncation)]
    let query_time_ms = start.elapsed().as_millis() as u64;
    file_results.results = merged.results;
    file_results.total = merged.total;
    file_results.query_time_ms = query_time_ms;
    Ok(file_results)
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
            std::process::Command::new(&app_name).arg(&path).spawn()
        };

        result.map_err(|e| format!("Failed to open with app '{app_name}': {e}"))?;
    } else {
        open::that(&path).map_err(|e| format!("Failed to open '{path}': {e}"))?;
    }

    record_history_for_path(&state.db, &path).await;
    Ok(())
}

#[tauri::command]
pub async fn start_indexing(
    paths: Vec<String>,
    app_handle: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let settings = state.settings.read().await.clone();
    let actual_paths = resolve_index_paths(paths, &settings);

    state
        .indexing
        .start(IndexingJob {
            paths: actual_paths,
            request: IndexRequest::Incremental,
            settings,
            app_handle,
            search_engine: state.search_engine.clone(),
            db: state.db.clone(),
            watcher: state.watcher.clone(),
        })
        .await
        .map_err(|e| {
            error!("Failed to start indexing: {}", e);
            e.to_string()
        })?;

    info!("Indexing started");
    Ok(())
}

#[tauri::command]
pub async fn stop_indexing(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state
        .indexing
        .stop(state.search_engine.clone())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_index_status(state: tauri::State<'_, AppState>) -> Result<IndexStatus, String> {
    let db = state.db.read().await;
    let files_indexed = db.get_indexed_file_count().map_err(|e| e.to_string())?;
    Ok(state.indexing.status(files_indexed).await)
}

#[tauri::command]
pub async fn get_settings(state: tauri::State<'_, AppState>) -> Result<AppSettings, String> {
    Ok(state.settings.read().await.clone())
}

#[tauri::command]
pub async fn update_settings(
    settings: AppSettings,
    app_handle: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let previous = state.settings.read().await.clone();

    crate::hotkey::validate_hotkey(&settings.hotkey).map_err(|e| e.to_string())?;

    if settings.launch_on_startup != previous.launch_on_startup {
        crate::set_autostart(settings.launch_on_startup, &app_handle)?;
    }

    if settings.hotkey != previous.hotkey {
        if let Err(err) = crate::hotkey::register_hotkey(&app_handle, &settings.hotkey, &state) {
            if settings.launch_on_startup != previous.launch_on_startup {
                crate::set_autostart(previous.launch_on_startup, &app_handle).ok();
            }
            return Err(err.to_string());
        }
    }

    let save_result = {
        let db = state.db.read().await;
        db.save_settings(&settings)
    };

    if let Err(err) = save_result {
        if settings.hotkey != previous.hotkey {
            crate::hotkey::register_hotkey(&app_handle, &previous.hotkey, &state).ok();
        }
        if settings.launch_on_startup != previous.launch_on_startup {
            crate::set_autostart(previous.launch_on_startup, &app_handle).ok();
        }
        return Err(err.to_string());
    }

    *state.settings.write().await = settings.clone();
    state
        .desktop_listener
        .set_enabled(app_handle.clone(), settings.enable_type_to_search);

    info!("Settings updated");
    app_handle.emit("settings-updated", settings).ok();
    Ok(())
}

#[tauri::command]
pub async fn launch_app(app_id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let apps = {
        let db = state.db.read().await;
        db.get_cached_apps().map_err(|e| e.to_string())?
    };

    let Some((_, name, path)) = apps.iter().find(|(id, _, _)| id == &app_id) else {
        return Err(format!("App with id '{app_id}' not found"));
    };

    crate::apps::launch_app(path)?;

    let item = HistoryItem {
        id: app_id,
        path: path.clone(),
        name: name.clone(),
        kind: ResultKind::App.as_str().to_string(),
        opened_at: chrono::Utc::now().timestamp(),
    };
    let db = state.db.read().await;
    db.add_history(&item).ok();

    Ok(())
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
pub async fn reindex_all(
    app_handle: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let settings = state.settings.read().await.clone();
    let paths = resolve_index_paths(Vec::new(), &settings);
    state
        .indexing
        .start(IndexingJob {
            paths,
            request: IndexRequest::Rebuild,
            settings,
            app_handle,
            search_engine: state.search_engine.clone(),
            db: state.db.clone(),
            watcher: state.watcher.clone(),
        })
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn scan_apps_now(state: tauri::State<'_, AppState>) -> Result<Vec<AppResult>, String> {
    state
        .app_scanner
        .scan_and_cache_apps()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_window_state(state: tauri::State<'_, AppState>) -> Result<Option<String>, String> {
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

fn merge_results(
    mut files: Vec<SearchResult>,
    apps: Vec<AppResult>,
    empty_query: bool,
    limit: u32,
    offset: u32,
) -> SearchResults {
    files.extend(apps.into_iter().map(|app| SearchResult {
        id: app.id,
        path: app.path,
        name: app.name,
        kind: ResultKind::App,
        score: if empty_query {
            1.0 + app.score
        } else {
            1000.0 + app.score
        },
        size: None,
        modified: None,
        icon: app.icon,
    }));

    files.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut seen = std::collections::HashSet::new();
    files.retain(|result| {
        let key = format!(
            "{}:{}",
            result.kind.as_str(),
            result.path.replace('\\', "/").to_lowercase()
        );
        seen.insert(key)
    });

    let total = files.len() as u64;
    let results = files
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect();

    SearchResults {
        results,
        total,
        query_time_ms: 0,
    }
}

fn resolve_index_paths(paths: Vec<String>, settings: &AppSettings) -> Vec<String> {
    if !paths.is_empty() {
        return paths;
    }
    if !settings.indexed_paths.is_empty() {
        return settings.indexed_paths.clone();
    }

    default_index_paths()
}

pub(crate) fn default_index_paths() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        crate::platform::get_all_windows_drives()
    }
    #[cfg(not(target_os = "windows"))]
    {
        vec!["/".to_string()]
    }
}

async fn record_history_for_path(db: &RwLock<SettingsDatabase>, path: &str) {
    let p = std::path::Path::new(path);
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let item = HistoryItem {
        id: path.to_string(),
        path: path.to_string(),
        name,
        kind: if p.is_dir() {
            ResultKind::Folder.as_str().to_string()
        } else {
            ResultKind::File.as_str().to_string()
        },
        opened_at: chrono::Utc::now().timestamp(),
    };

    let db = db.read().await;
    db.add_history(&item).ok();
}
