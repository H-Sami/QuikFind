// REASON: all lock guards in this codebase are held briefly for atomic operations;
// adding explicit drop() calls everywhere would add noise without meaningful benefit
#![allow(clippy::significant_drop_tightening)]

pub mod apps;
pub mod commands;
pub mod desktop_listener;
pub mod error;
pub mod indexer;
pub mod models;
pub mod plugins;
pub mod search;
pub mod settings;
pub mod watcher;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
struct WindowState {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

use commands::AppState;
use plugins::PluginRegistry;
use search::SearchEngine;
use settings::SettingsDatabase;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tauri::{Emitter, LogicalPosition, LogicalSize, Manager};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::WindowEvent;
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

/// # Panics
/// Panics if the Tauri application fails to start.
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
    // SAFETY: Tauri startup failure is unrecoverable; panic with message
    if let Err(e) = result {
        panic!("QuikFind failed to start: {e}");
    }
}

fn init_search_engine(index_dir: &Path) -> std::result::Result<Arc<SearchEngine>, Box<dyn std::error::Error>> {
    let search_engine = Arc::new(SearchEngine::new());
    search_engine.initialize_index(index_dir)?;
    Ok(search_engine)
}

// REASON: setup_app handles multiple initialization concerns; breaking it up would not improve clarity
#[allow(clippy::too_many_lines)]
fn setup_app(app: &mut tauri::App) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let config_dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("quikfind");

    std::fs::create_dir_all(&config_dir).ok();

    let index_dir = config_dir.join("index");
    std::fs::create_dir_all(&index_dir).ok();

    let db = SettingsDatabase::open(&config_dir)?;
    let settings = db.load_settings().unwrap_or_default();
    info!("Loaded settings: {:?}", settings);
    let db = Arc::new(RwLock::new(db));

    let settings_arc = Arc::new(RwLock::new(settings.clone()));

    let search_engine = init_search_engine(&index_dir)?;
    let watcher = Arc::new(watcher::FileWatcher::new());
    let app_scanner = Arc::new(apps::AppScanner::new(db.clone()));
    let plugin_registry = Arc::new(Mutex::new(PluginRegistry::new(&config_dir)));

    app.manage(AppState {
        search_engine: search_engine.clone(),
        db: db.clone(),
        settings: settings_arc,
        watcher: watcher.clone(),
        app_scanner: app_scanner.clone(),
        plugin_registry,
        indexing_handle: Arc::new(Mutex::new(None)),
        is_indexing: Arc::new(AtomicBool::new(false)),
    });

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        if let Err(e) = app_scanner.scan_and_cache_apps().await {
            error!("Failed to scan apps: {}", e);
        }
    });

    let initial_paths = if settings.indexed_paths.is_empty() {
        #[cfg(target_os = "windows")]
        {
            commands::get_all_windows_drives()
        }
        #[cfg(not(target_os = "windows"))]
        {
            vec!["/".to_string()]
        }
    } else {
        settings.indexed_paths.clone()
    };

    spawn_initial_indexing(
        search_engine,
        db,
        watcher,
        initial_paths,
        // REASON: separate clone needed because settings is moved into spawn_initial_indexing below
        #[allow(clippy::redundant_clone)]
        settings.excluded_patterns.clone(),
        app.handle().clone(),
    );

    // === CLEAN SYSTEM TRAY ===
    let tray_menu = Menu::with_items(
        app.handle(),
        &[
            &MenuItem::with_id(app.handle(), "show", "Show QuikFind", true, None::<&str>)?,
            &MenuItem::with_id(app.handle(), "settings", "Options", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app.handle())?,
            &MenuItem::with_id(app.handle(), "quit", "Quit", true, None::<&str>)?,
        ],
    )?;

    let _tray = TrayIconBuilder::new()
        .icon(app.default_window_icon().cloned().unwrap())
        .tooltip("QuikFind")
        .menu(&tray_menu)
        .on_menu_event(|app, event| {
            match event.id.as_ref() {
                "show" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                "settings" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                        let _ = window.emit("open-settings", ());
                    }
                }
                "quit" => {
                    app.exit(0);
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, .. } = event {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    if window.is_visible().unwrap_or(false) {
                        let _ = window.hide();
                    } else {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
            }
        })
        .build(app)?;

    // Prevent closing (minimize to tray)
    if let Some(window) = app.get_webview_window("main") {
        let window_clone = window.clone();
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window_clone.hide();
            }
        });
    }

    // === DESKTOP AUTO-TRIGGER (Type to Search) ===
    desktop_listener::start_desktop_listener(app.handle().clone());

    // === REGISTER GLOBAL HOTKEY FROM SETTINGS ===
    register_hotkey(app.handle(), &settings.hotkey);

    // === WINDOW STATE RESTORATION ===
    if let Some(window) = app.get_webview_window("main") {
        let state_json = {
            let db = app.state::<AppState>().db.clone();
            let db_guard = db.blocking_read();
            db_guard.get_window_state()
        };

        if let Ok(Some(saved_state)) = state_json {
            if let Ok(saved) = serde_json::from_str::<WindowState>(&saved_state) {
                let monitor = window.current_monitor().ok().flatten();

                if let Some(monitor) = monitor {
                    let monitor_pos = monitor.position();
                    let monitor_size = monitor.size();

                    let is_valid = saved.x >= (f64::from(monitor_pos.x)) - 100.0
                        && saved.y >= (f64::from(monitor_pos.y)) - 100.0
                        && saved.x < (f64::from(monitor_pos.x) + f64::from(monitor_size.width))
                        && saved.y < (f64::from(monitor_pos.y) + f64::from(monitor_size.height));

                    if is_valid {
                        let _ = window.set_position(LogicalPosition::new(saved.x, saved.y));
                        let _ = window.set_size(LogicalSize::new(saved.width, saved.height));
                    }
                }
            }
        }

        // Save window state on move and resize
        let window_clone = window.clone();
        let db_clone = app.state::<AppState>().db.clone();

        window.on_window_event(move |event| {
            if let WindowEvent::Moved(_) | WindowEvent::Resized(_) = event {
                if let (Ok(pos), Ok(size)) =
                    (window_clone.outer_position(), window_clone.outer_size())
                {
                    let state = WindowState {
                        x: f64::from(pos.x),
                        y: f64::from(pos.y),
                        width: f64::from(size.width),
                        height: f64::from(size.height),
                    };

                    if let Ok(json) = serde_json::to_string(&state) {
                        let db = db_clone.blocking_read();
                        let _ = db.save_window_state(&json);
                    }
                }
            }
        });
    }

    info!("QuikFind setup complete");
    Ok(())
}

pub fn register_hotkey(app: &tauri::AppHandle, hotkey_str: &str) {
    let _ = app.global_shortcut().unregister_all();

    if let Ok(shortcut) = hotkey_str.parse::<Shortcut>() {
        let app = app.clone();

        if let Err(e) = app.global_shortcut().on_shortcut(shortcut, move |handle, _shortcut, event| {
            // Only trigger on key press, ignore key release
            if event.state == ShortcutState::Pressed {
                if let Some(window) = handle.get_webview_window("main") {
                    if window.is_visible().unwrap_or(false) {
                        let _ = window.hide();
                    } else {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
            }
        }) {
            error!("Failed to register global hotkey: {}", e);
        }
    } else {
        error!("Failed to parse hotkey: {}", hotkey_str);
    }
}

/// Sets whether `QuikFind` should launch automatically on Windows startup.
///
/// # Errors
/// Returns an error if the autostart configuration fails.
pub fn set_autostart(enabled: bool, app: &tauri::AppHandle) -> Result<(), String> {
    let autostart_manager = app.autolaunch();

    if enabled {
        autostart_manager.enable().map_err(|e| e.to_string())?;
    } else {
        autostart_manager.disable().map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn spawn_initial_indexing(
    search_engine: Arc<SearchEngine>,
    db: Arc<RwLock<SettingsDatabase>>,
    watcher: Arc<watcher::FileWatcher>,
    indexed_paths: Vec<String>,
    excluded_patterns: Vec<String>,
    app_handle: tauri::AppHandle,
) {
    let is_indexing = app_handle.state::<AppState>().is_indexing.clone();
    let indexing_handle = app_handle.state::<AppState>().indexing_handle.clone();

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        let (progress_tx, mut progress_rx) =
            tokio::sync::mpsc::unbounded_channel();

        is_indexing.store(true, std::sync::atomic::Ordering::Relaxed);

        let indexer =
            indexer::Indexer::new(search_engine.clone(), db.clone());

        let handle = tokio::spawn(async move {
            if let Err(e) = indexer
                .index_paths(indexed_paths.clone(), &excluded_patterns, progress_tx)
                .await
            {
                error!("Initial indexing failed: {}", e);
            }

            if let Err(e) = watcher
                .update_watched_paths(
                    &indexed_paths,
                    search_engine.clone(),
                    db.clone(),
                )
            {
                error!("Watcher restart after indexing: {}", e);
            }

            is_indexing.store(false, std::sync::atomic::Ordering::Relaxed);
        });

        let mut h = indexing_handle.lock().await;
        *h = Some(handle);
        drop(h);

        while let Some(progress) = progress_rx.recv().await {
            let status = crate::models::IndexStatus {
                is_indexing: true,
                files_indexed: progress.files_indexed,
                total_files: progress.total_files,
                progress_percent: if progress.total_files > 0 {
                    {
                        // REASON: float cast for progress display; sub-integer precision is acceptable
                        #[allow(clippy::cast_precision_loss)]
                        let ratio = progress.files_indexed as f32 / progress.total_files as f32;
                        ratio * 100.0
                    }
                } else {
                    0.0
                },
                last_updated: chrono::Utc::now().timestamp(),
                errors: progress.errors,
            };
            app_handle.emit("index-progress", status).ok();
        }
        app_handle
            .emit(
                "index-progress",
                crate::models::IndexStatus {
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
}
