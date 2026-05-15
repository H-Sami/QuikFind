use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tracing::warn;

use crate::commands::AppState;
use crate::error::{QuikFindError, Result};

pub fn register_hotkey(app: &AppHandle, hotkey_str: &str, state: &AppState) -> Result<()> {
    let shortcut = validate_hotkey(hotkey_str)?;
    let mut active = state.active_hotkey.lock();

    match shortcut {
        None => {
            if let Some(old) = active.take() {
                app.global_shortcut().unregister(old).map_err(|e| {
                    QuikFindError::Generic(format!("Failed to unregister hotkey: {e}"))
                })?;
            }
            Ok(())
        }
        Some(new_shortcut) => {
            if active.as_ref().is_some_and(|old| *old == new_shortcut) {
                return Ok(());
            }

            app.global_shortcut()
                .on_shortcut(new_shortcut, move |handle, _sc, event| {
                    if event.state == ShortcutState::Pressed {
                        if let Some(window) = handle.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .map_err(|e| {
                    QuikFindError::Generic(format!("Failed to register hotkey '{hotkey_str}': {e}"))
                })?;

            if let Some(old) = active.take() {
                if let Err(e) = app.global_shortcut().unregister(old) {
                    warn!(
                        "Registered new hotkey but failed to unregister old hotkey: {}",
                        e
                    );
                }
            }
            *active = Some(new_shortcut);
            Ok(())
        }
    }
}

pub(crate) fn validate_hotkey(hotkey_str: &str) -> Result<Option<Shortcut>> {
    let trimmed = hotkey_str.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let has_primary_key = trimmed
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .any(|part| !is_modifier_name(part));
    if !has_primary_key {
        return Err(QuikFindError::Settings(
            "Hotkey must include a non-modifier key".to_string(),
        ));
    }

    trimmed
        .parse::<Shortcut>()
        .map(Some)
        .map_err(|e| QuikFindError::Settings(format!("Invalid hotkey '{hotkey_str}': {e}")))
}

fn is_modifier_name(part: &str) -> bool {
    matches!(
        part.to_ascii_lowercase().as_str(),
        "ctrl"
            | "control"
            | "cmd"
            | "command"
            | "cmdorctrl"
            | "shift"
            | "alt"
            | "option"
            | "meta"
            | "super"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_hotkey_rejects_modifier_only_shortcuts() {
        assert!(validate_hotkey("Ctrl+Shift").is_err());
        assert!(validate_hotkey("CmdOrCtrl").is_err());
    }

    #[test]
    fn validate_hotkey_accepts_empty_as_disabled() {
        assert!(validate_hotkey("").unwrap().is_none());
    }

    #[test]
    fn validate_hotkey_accepts_default_shortcut() {
        assert!(validate_hotkey("CmdOrCtrl+Space").unwrap().is_some());
    }
}
