use tauri::{App, Emitter, Manager};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

pub fn setup_tray(app: &mut App) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let tray_menu = Menu::with_items(app.handle(), &[
        &MenuItem::with_id(app.handle(), "show", "Show QuikFind", true, None::<&str>)?,
        &MenuItem::with_id(app.handle(), "settings", "Options", true, None::<&str>)?,
        &PredefinedMenuItem::separator(app.handle())?,
        &MenuItem::with_id(app.handle(), "quit", "Quit", true, None::<&str>)?,
    ])?;

    let icon = app.default_window_icon()
        .cloned()
        .ok_or("No default window icon configured")?;

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("QuikFind")
        .menu(&tray_menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
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
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}
