use crate::error::Result;
use crate::models::AppResult;
use crate::settings::SettingsDatabase;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

pub struct AppScanner {
    db: Arc<RwLock<SettingsDatabase>>,
    // REASON: reserved for configurable cache TTL in future
    #[allow(dead_code)]
    cache_duration: Duration,
}

impl AppScanner {
    pub const fn new(db: Arc<RwLock<SettingsDatabase>>) -> Self {
        Self {
            db,
            cache_duration: Duration::from_secs(86400),
        }
    }

    /// Searches cached apps by query.
    ///
    /// # Errors
    /// Returns an error if the database cannot be read.
    pub async fn search_apps(&self, query: &str) -> Result<Vec<AppResult>> {
        let apps = self.get_or_scan_apps().await?;

        if query.trim().is_empty() {
            return Ok(apps.into_iter().take(20).collect());
        }

        let query_lower = query.to_lowercase();
        let mut scored: Vec<(f32, AppResult)> = apps
            .into_iter()
            .map(|app| {
                let name_lower = app.name.to_lowercase();
                let exact_prefix = if name_lower.starts_with(&query_lower) {
                    2.0
                } else {
                    0.0
                };
                let contains = if name_lower.contains(&query_lower) {
                    1.0
                } else {
                    0.0
                };
                let fuzzy = compute_simple_fuzzy(&query_lower, &name_lower);
                let score = exact_prefix * 3.0 + fuzzy * 2.0 + contains;
                (score, app)
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.retain(|(s, _)| *s > 0.0);

        Ok(scored.into_iter().map(|(_, app)| app).collect())
    }

    async fn get_or_scan_apps(&self) -> Result<Vec<AppResult>> {
        {
            let db = self.db.read().await;
            let cached = db.get_cached_apps()?;
            if !cached.is_empty() {
                let apps: Vec<AppResult> = cached
                    .into_iter()
                    .map(|(id, name, path)| AppResult {
                        id,
                        name,
                        path,
                        icon: None,
                        score: 0.0,
                    })
                    .collect();
                return Ok(apps);
            }
        }

        self.scan_and_cache_apps().await
    }

    /// Scans for installed applications and caches them.
    ///
    /// # Errors
    /// Returns an error if scanning fails.
    pub async fn scan_and_cache_apps(&self) -> Result<Vec<AppResult>> {
        let start = Instant::now();
        let mut apps = Vec::new();

        #[cfg(target_os = "windows")]
        {
            apps.extend(scan_windows_apps());
        }

        #[cfg(target_os = "macos")]
        {
            apps.extend(scan_macos_apps()?);
        }

        #[cfg(target_os = "linux")]
        {
            apps.extend(scan_linux_apps()?);
        }

        let db = self.db.read().await;
        for app in &apps {
            db.cache_app(&app.id, &app.name, &app.path, None)
                .ok();
        }

        info!(
            "Scanned and cached {} apps in {:?}",
            apps.len(),
            start.elapsed()
        );

        Ok(apps)
    }
}

// REASON: fuzzy score is a relative ranking, not an exact value; float cast is acceptable
#[allow(clippy::cast_precision_loss)]
fn compute_simple_fuzzy(query: &str, name: &str) -> f32 {
    let query_chars: Vec<char> = query.chars().collect();
    let name_chars: Vec<char> = name.chars().collect();

    let mut qi = 0;
    let mut score = 0.0;
    let mut consecutive = 0.0;

    for &nc in &name_chars {
        if qi < query_chars.len() && nc.eq_ignore_ascii_case(&query_chars[qi])
        {
            qi += 1;
            consecutive += 1.0;
            score += consecutive;
        } else {
            consecutive = 0.0;
        }
    }

    if qi == query_chars.len() {
        score / (name_chars.len() as f32)
    } else {
        0.0
    }
}

#[cfg(target_os = "windows")]
fn scan_windows_apps() -> Vec<AppResult> {
    let mut apps = Vec::new();

    let start_menu_paths = vec![
        PathBuf::from(
            std::env::var("PROGRAMDATA")
                .unwrap_or_else(|_| "C:\\ProgramData".to_string()),
        )
        .join("Microsoft\\Windows\\Start Menu\\Programs"),
        PathBuf::from(
            std::env::var("APPDATA")
                .unwrap_or_else(|_| {
                    format!(
                        "{}\\AppData\\Roaming",
                        std::env::var("USERPROFILE")
                            .unwrap_or_else(|_| "C:\\Users\\Default".to_string())
                    )
                }),
        )
        .join("Microsoft\\Windows\\Start Menu\\Programs"),
    ];

    for start_path in &start_menu_paths {
        if !start_path.exists() {
            continue;
        }
        collect_lnk_files(start_path, &mut apps, 0, 3);
    }

    let local_app_data = std::env::var("LOCALAPPDATA")
        .unwrap_or_else(|_| "C:\\Users\\Default\\AppData\\Local".to_string());
    let known_folders = vec![
        "C:\\Program Files",
        "C:\\Program Files (x86)",
        &local_app_data,
    ];

    for folder in &known_folders {
        let dir = PathBuf::from(folder);
        if !dir.exists() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(dir.join("Microsoft\\WindowsApps")) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .extension()
                    .is_some_and(|ext| ext == "exe")
                {
                    let name = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let id = Uuid::new_v4().to_string();
                    apps.push(AppResult {
                        id,
                        name,
                        path: path.to_string_lossy().to_string(),
                        icon: None,
                        score: 0.0,
                    });
                }
            }
        }
    }

    apps
}

#[cfg(target_os = "windows")]
fn collect_lnk_files(dir: &PathBuf, apps: &mut Vec<AppResult>, depth: u32, max_depth: u32) {
    if depth > max_depth {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let next_depth: u32 = depth + 1;
                collect_lnk_files(&path, apps, next_depth, max_depth);
            } else if let Some(ext) = path.extension() {
                if ext == "lnk" || ext == "exe" {
                    let name = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let id = uuid::Uuid::new_v4().to_string();
                    apps.push(AppResult {
                        id,
                        name,
                        path: path.to_string_lossy().to_string(),
                        icon: None,
                        score: 0.0,
                    });
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn scan_macos_apps() -> Result<Vec<AppResult>> {
    let mut apps = Vec::new();
    let search_paths = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/Applications/Utilities"),
        PathBuf::from(
            dirs::home_dir()
                .unwrap_or_default()
                .join("Applications"),
        ),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
    ];

    for search_path in &search_paths {
        if !search_path.exists() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(search_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .extension()
                    .map(|ext| ext == "app")
                    .unwrap_or(false)
                {
                    let name = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let id = Uuid::new_v4().to_string();

                    let icon = extract_macos_icon(&path);

                    apps.push(AppResult {
                        id,
                        name,
                        path: path.to_string_lossy().to_string(),
                        icon,
                        score: 0.0,
                    });
                }
            }
        }
    }

    Ok(apps)
}

#[cfg(target_os = "macos")]
fn extract_macos_icon(app_path: &PathBuf) -> Option<String> {
    let icon_path = app_path.join("Contents/Resources");
    if !icon_path.exists() {
        return None;
    }
    if let Ok(entries) = std::fs::read_dir(&icon_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "icns" {
                    if let Ok(data) = std::fs::read(&path) {
                        if data.len() < 1024 * 1024 {
                            return Some(
                                format!(
                                    "data:image/x-icns;base64,{}",
                                    base64_encoding(&data)
                                ),
                            );
                        }
                    }
                }
                if ext == "png" && path.to_string_lossy().contains("icon") {
                    if let Ok(data) = std::fs::read(&path) {
                        if data.len() < 512 * 1024 {
                            return Some(
                                format!(
                                    "data:image/png;base64,{}",
                                    base64_encoding(&data)
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
    None
}

#[must_use]
pub fn base64_encoding(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

#[cfg(target_os = "linux")]
fn scan_linux_apps() -> Result<Vec<AppResult>> {
    let mut apps = Vec::new();
    let desktop_dirs = vec![
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
        dirs::home_dir()
            .unwrap_or_default()
            .join(".local/share/applications"),
        PathBuf::from("/var/lib/snapd/desktop/applications"),
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
    ];

    for dir in &desktop_dirs {
        if !dir.exists() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .extension()
                    .map(|ext| ext == "desktop")
                    .unwrap_or(false)
                {
                    if let Some(app) = parse_desktop_file(&path) {
                        apps.push(app);
                    }
                }
            }
        }
    }

    apps.sort_by(|a, b| a.name.cmp(&b.name));
    apps.dedup_by(|a, b| a.name == b.name);

    Ok(apps)
}

#[cfg(target_os = "linux")]
fn parse_desktop_file(path: &PathBuf) -> Option<AppResult> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut name = None;
    let mut exec = None;
    let mut no_display = false;
    let mut term = false;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(val) = line.strip_prefix("NoDisplay=") {
            no_display = val.trim() == "true";
        }
        if let Some(val) = line.strip_prefix("Name=") {
            if name.is_none() {
                name = Some(val.trim().to_string());
            }
        }
        if let Some(val) = line.strip_prefix("Exec=") {
            exec = Some(val.trim().to_string());
        }
        if let Some(val) = line.strip_prefix("Terminal=") {
            term = val.trim() == "true";
        }
    }

    if no_display || name.is_none() || exec.is_none() {
        return None;
    }

    let exec_str = exec.unwrap_or_default();
    let cmd = exec_str
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches('"')
        .to_string();

    if cmd.is_empty() {
        return None;
    }

    let id = Uuid::new_v4().to_string();
    Some(AppResult {
        id,
        name: name.unwrap_or_default(),
        path: cmd,
        icon: None,
        score: 0.0,
    })
}

/// Launches a file or app at the given path.
///
/// # Errors
/// Returns an error if the path cannot be opened.
pub fn launch_app(path: &str) -> std::result::Result<(), String> {
    let result = open::that(path);
    match result {
        Ok(()) => {
            debug!("Launched: {path}");
            Ok(())
        }
        Err(e) => {
            let err_msg = format!("Failed to launch '{path}': {e}");
            warn!("{err_msg}");
            Err(err_msg)
        }
    }
}
