#![allow(clippy::significant_drop_tightening, clippy::missing_panics_doc)]

mod apps;
pub mod commands;
mod desktop_listener;
pub mod error;
mod hotkey;
mod indexer;
pub mod models;
mod platform;
pub mod plugins;
mod search;
mod settings;
mod tray;
mod watcher;

use commands::AppState;
use error::{QuikFindError, Result};
use models::IndexStatus;
use plugins::PluginRegistry;
use search::SearchEngine;
use settings::SettingsDatabase;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use tauri::{App, AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WindowEvent};
use tauri_plugin_autostart::{ManagerExt, MacosLauncher};
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
struct WindowState {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("quikfind=info, tantivy=warn")),
        )
        .with_file(true)
        .with_line_number(true)
        .init();

    info!("Starting QuikFind v{}", env!("CARGO_PKG_VERSION"));

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .setup(setup_app)
        .invoke_handler(tauri::generate_handler![
            commands::search,
            commands::open_path,
            commands::start_indexing,
            commands::stop_indexing,
            commands::get_index_status,
            commands::get_settings,
            commands::update_settings,
            commands::search_apps,
            commands::launch_app,
            commands::get_history,
            commands::add_to_history,
            commands::reindex_all,
            commands::scan_apps_now,
            commands::get_plugins,
            commands::get_window_state,
            commands::set_autostart,
            commands::save_window_state,
        ])
        .run(tauri::generate_context!());
    if let Err(e) = result {
        panic!("QuikFind failed to start: {e}");
    }
}

async fn run_indexing(
    paths: Vec<String>,
    state: &AppState,
    app_handle: &AppHandle,
) -> Result<()> {
    if state.is_indexing.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(QuikFindError::Generic("Indexing is already in progress".into()));
    }

    let settings = {
        let db = state.db.read().await;
        db.load_settings()?
    };

    state.is_indexing.store(true, std::sync::atomic::Ordering::Relaxed);

    let search_engine = state.search_engine.clone();
    let db = state.db.clone();
    let watcher = state.watcher.clone();
    let is_indexing = state.is_indexing.clone();
    let indexing_handle = state.indexing_handle.clone();
    let excluded = settings.excluded_patterns.clone();
    let app_handle2 = app_handle.clone();
    let app_handle3 = app_handle.clone();
    let phase2_paths = paths.clone();

    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let progress_tx2 = progress_tx.clone();

    let handle = tokio::spawn(async move {
        // Phase 1: metadata-only — fast, completes in seconds
        let indexer = indexer::Indexer::new(search_engine.clone(), db.clone());

        if let Err(e) = indexer.index_paths(paths.clone(), &excluded, progress_tx).await {
            error!("Phase 1 indexing failed: {}", e);
        }

        // Phase 1 complete: emit event so frontend knows search is available
        app_handle2
            .emit(
                "index-phase1-complete",
                models::IndexStatus {
                    is_indexing: true,
                    files_indexed: 0,
                    total_files: 0,
                    progress_percent: 100.0,
                    last_updated: chrono::Utc::now().timestamp(),
                    errors: Vec::new(),
                },
            )
            .ok();

        // Start file watcher now so live changes are tracked during Phase 2
        if let Err(e) = watcher
            .update_watched_paths(&phase2_paths, search_engine.clone(), db.clone())
        {
            error!("Failed to update watched paths: {}", e);
        }

        // Phase 2: content enrichment — background, does not block user
        let se = search_engine.clone();
        let db2 = db.clone();
        tokio::spawn(async move {
            let indexer2 = indexer::Indexer::new(se, db2);
            if let Err(e) = indexer2.enrich_content(progress_tx2).await {
                error!("Content enrichment failed: {}", e);
            }
            is_indexing.store(false, std::sync::atomic::Ordering::Relaxed);
        });
    });

    tokio::spawn(async move {
        while let Some(progress) = progress_rx.recv().await {
            #[allow(clippy::cast_precision_loss)]
            let progress_percent = if progress.total_files > 0 {
                (progress.files_indexed as f32 / progress.total_files as f32) * 100.0
            } else {
                0.0
            };

            app_handle3
                .emit(
                    "index-progress",
                    IndexStatus {
                        is_indexing: true,
                        files_indexed: progress.files_indexed,
                        total_files: progress.total_files,
                        progress_percent,
                        last_updated: chrono::Utc::now().timestamp(),
                        errors: progress.errors,
                    },
                )
                .ok();
        }

        app_handle3
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

    Ok(())
}

fn init_app_state(app: &mut App, config_dir: &Path) -> std::result::Result<models::AppSettings, Box<dyn std::error::Error>> {
    let db = SettingsDatabase::open(config_dir)?;
    let settings = db.load_settings().unwrap_or_default();
    let db = Arc::new(RwLock::new(db));

    let search_engine = {
        let engine = Arc::new(SearchEngine::new());
        engine.initialize_index(&config_dir.join("index"))?;
        engine
    };

    let settings_arc = Arc::new(RwLock::new(settings.clone()));
    let watcher = Arc::new(watcher::FileWatcher::new());
    let app_scanner = Arc::new(apps::AppScanner::new(db.clone()));
    let plugin_registry = Arc::new(Mutex::new(PluginRegistry::new(config_dir)));

    app.manage(AppState {
        search_engine,
        db,
        settings: settings_arc,
        watcher,
        app_scanner,
        plugin_registry,
        indexing_handle: Arc::new(Mutex::new(None)),
        is_indexing: Arc::new(AtomicBool::new(false)),
        active_hotkey: Arc::new(parking_lot::Mutex::new(None)),
    });

    Ok(settings)
}

fn setup_window_behavior(app: &App) {
    if let Some(window) = app.get_webview_window("main") {
        let window_clone = window.clone();
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window_clone.hide();
            }
        });
    }
}

fn restore_window_state(app: &App) {
    let Some(window) = app.get_webview_window("main") else { return };

    let db = app.state::<AppState>().db.clone();
    let window_for_restore = window.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let db_guard = db.read().await;
        let Ok(Some(json)) = db_guard.get_window_state() else { return };
        drop(db_guard);
        let Ok(saved) = serde_json::from_str::<WindowState>(&json) else { return };
        let Ok(Some(monitor)) = window_for_restore.current_monitor() else { return };
        let mp = monitor.position();
        let ms = monitor.size();
        let on_screen = saved.x >= f64::from(mp.x) - 100.0
            && saved.y >= f64::from(mp.y) - 100.0
            && saved.x < f64::from(mp.x) + f64::from(ms.width)
            && saved.y < f64::from(mp.y) + f64::from(ms.height);
        if on_screen {
            let _ = window_for_restore.set_position(LogicalPosition::new(saved.x, saved.y));
            let _ = window_for_restore.set_size(LogicalSize::new(saved.width, saved.height));
        }
    });

    let db_save = app.state::<AppState>().db.clone();
    let window_for_save = window.clone();
    window.on_window_event(move |event| {
        if !matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_)) { return; }
        let Ok(pos)  = window_for_save.outer_position() else { return };
        let Ok(size) = window_for_save.outer_size()     else { return };
        let state = WindowState {
            x: f64::from(pos.x), y: f64::from(pos.y),
            width: f64::from(size.width), height: f64::from(size.height),
        };
        let Ok(json) = serde_json::to_string(&state) else { return };
        let db = db_save.clone();
        tauri::async_runtime::spawn(async move {
            let guard = db.read().await;
            let _ = guard.save_window_state(&json);
        });
    });
}

fn setup_app(app: &mut App) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let config_dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("quikfind");

    std::fs::create_dir_all(&config_dir).ok();
    std::fs::create_dir_all(config_dir.join("index")).ok();

    let settings = init_app_state(app, &config_dir)?;

    let app_scanner = app.state::<AppState>().app_scanner.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if let Err(e) = app_scanner.scan_and_cache_apps().await {
            error!("Failed to scan apps: {}", e);
        }
    });

    let initial_paths = if settings.indexed_paths.is_empty() {
        #[cfg(target_os = "windows")]
        { platform::get_all_windows_drives() }
        #[cfg(not(target_os = "windows"))]
        { vec!["/".to_string()] }
    } else {
        settings.indexed_paths.clone()
    };

    // Initialize Rayon with one fewer thread than logical cores, leaving one for Tokio.
    let rayon_threads = (num_cpus::get().saturating_sub(1)).max(1);
    rayon::ThreadPoolBuilder::new()
        .num_threads(rayon_threads)
        .build_global()
        .ok();

    let app_handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        // Allow the Tauri window to finish rendering before Phase 1 indexing begins.
        // Phase 1 is metadata-only and completes in seconds, so 500ms is sufficient.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let state = app_handle.state::<AppState>();
        if let Err(e) = run_indexing(initial_paths, &state, &app_handle).await {
            error!("Initial indexing failed: {}", e);
        }
    });

    tray::setup_tray(app)?;
    setup_window_behavior(app);
    desktop_listener::start_desktop_listener(app.handle().clone());

    let state = app.state::<AppState>();
    hotkey::register_hotkey(app.handle(), &settings.hotkey, &state);
    restore_window_state(app);

    info!("QuikFind setup complete");
    Ok(())
}

#[allow(clippy::missing_errors_doc)]
pub fn set_autostart(enabled: bool, app: &AppHandle) -> std::result::Result<(), String> {
    let autostart_manager = app.autolaunch();

    if enabled {
        autostart_manager.enable().map_err(|e| e.to_string())?;
    } else {
        autostart_manager.disable().map_err(|e| e.to_string())?;
    }

    Ok(())
}
