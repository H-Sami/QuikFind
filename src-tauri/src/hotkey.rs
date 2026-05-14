use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tracing::error;

use crate::commands::AppState;

pub fn register_hotkey(app: &AppHandle, hotkey_str: &str, state: &AppState) {
    let mut active = state.active_hotkey.lock();
    if let Some(old) = active.take() {
        let _ = app.global_shortcut().unregister(old);
    }

    let Ok(shortcut) = hotkey_str.parse::<Shortcut>() else {
        error!("Failed to parse hotkey: {}", hotkey_str);
        return;
    };

    match app.global_shortcut().on_shortcut(shortcut, move |handle, _sc, event| {
        if event.state == ShortcutState::Pressed {
            if let Some(window) = handle.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
    }) {
        Ok(()) => { *active = Some(shortcut); }
        Err(e) => { error!("Failed to register hotkey '{}': {}", hotkey_str, e); }
    }
}
