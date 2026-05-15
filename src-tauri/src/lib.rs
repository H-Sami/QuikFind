#![allow(clippy::significant_drop_tightening, clippy::missing_panics_doc)]

mod apps;
pub mod commands;
mod desktop_listener;
pub mod error;
mod hotkey;
mod indexer;
mod indexing;
pub mod models;
mod platform;
pub mod plugins;
mod search;
mod settings;
mod tray;
mod watcher;

use commands::AppState;
use indexing::{IndexRequest, IndexingJob};
use plugins::PluginRegistry;
use search::SearchEngine;
use settings::SettingsDatabase;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tauri::{App, AppHandle, LogicalPosition, LogicalSize, Manager, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
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
            commands::launch_app,
            commands::get_history,
            commands::reindex_all,
            commands::scan_apps_now,
            commands::get_plugins,
            commands::get_window_state,
            commands::save_window_state,
        ])
        .run(tauri::generate_context!());
    if let Err(e) = result {
        panic!("QuikFind failed to start: {e}");
    }
}

fn init_app_state(
    app: &mut App,
    config_dir: &Path,
) -> std::result::Result<models::AppSettings, Box<dyn std::error::Error>> {
    let db = SettingsDatabase::open(config_dir)?;
    let settings = db.load_settings().unwrap_or_default();
    let db = Arc::new(RwLock::new(db));

    let search_engine = {
        let engine = Arc::new(SearchEngine::new());
        engine.initialize_index(&config_dir.join("index"))?;
        engine
    };

    app.manage(AppState {
        search_engine,
        db: db.clone(),
        settings: Arc::new(RwLock::new(settings.clone())),
        watcher: Arc::new(watcher::FileWatcher::new()),
        app_scanner: Arc::new(apps::AppScanner::new(db)),
        plugin_registry: Arc::new(Mutex::new(PluginRegistry::new(config_dir))),
        indexing: Arc::new(indexing::IndexingSupervisor::new()),
        desktop_listener: Arc::new(desktop_listener::DesktopListener::new()),
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
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    let db = app.state::<AppState>().db.clone();
    let window_for_restore = window.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let db_guard = db.read().await;
        let Ok(Some(json)) = db_guard.get_window_state() else {
            return;
        };
        drop(db_guard);
        let Ok(saved) = serde_json::from_str::<WindowState>(&json) else {
            return;
        };
        let Ok(Some(monitor)) = window_for_restore.current_monitor() else {
            return;
        };
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
        if !matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_)) {
            return;
        }
        let Ok(pos) = window_for_save.outer_position() else {
            return;
        };
        let Ok(size) = window_for_save.outer_size() else {
            return;
        };
        let state = WindowState {
            x: f64::from(pos.x),
            y: f64::from(pos.y),
            width: f64::from(size.width),
            height: f64::from(size.height),
        };
        let Ok(json) = serde_json::to_string(&state) else {
            return;
        };
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
        commands::default_index_paths()
    } else {
        settings.indexed_paths.clone()
    };

    let rayon_threads = (num_cpus::get().saturating_sub(1)).max(1);
    rayon::ThreadPoolBuilder::new()
        .num_threads(rayon_threads)
        .build_global()
        .ok();

    let app_handle = app.handle().clone();
    let launch_settings = settings.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let state = app_handle.state::<AppState>();
        let indexed_count = {
            let db = state.db.read().await;
            db.get_indexed_file_count().unwrap_or(0)
        };

        if indexed_count == 0 {
            if let Err(e) = state
                .indexing
                .start(IndexingJob {
                    paths: initial_paths,
                    request: IndexRequest::Rebuild,
                    settings: launch_settings,
                    app_handle: app_handle.clone(),
                    search_engine: state.search_engine.clone(),
                    db: state.db.clone(),
                    watcher: state.watcher.clone(),
                })
                .await
            {
                error!("Initial indexing failed: {}", e);
            }
        } else if let Err(e) = state.watcher.update_watched_paths(
            &initial_paths,
            &launch_settings.excluded_patterns,
            state.search_engine.clone(),
            state.db.clone(),
        ) {
            error!("Failed to start watcher on existing index: {}", e);
        }
    });

    tray::setup_tray(app)?;
    setup_window_behavior(app);

    let state = app.state::<AppState>();
    state
        .desktop_listener
        .set_enabled(app.handle().clone(), settings.enable_type_to_search);
    if let Err(e) = hotkey::register_hotkey(app.handle(), &settings.hotkey, &state) {
        error!("Failed to register hotkey: {}", e);
    }
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
