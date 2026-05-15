use crate::platform::is_on_desktop;
use rdev::{listen, EventType, Key};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use tauri::{AppHandle, Emitter, Manager};
use tracing::{error, info};

#[derive(Default)]
pub struct DesktopListener {
    enabled: Arc<AtomicBool>,
    started: AtomicBool,
}

impl DesktopListener {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_enabled(&self, app: AppHandle, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        if enabled {
            self.start_once(app);
        }
    }

    fn start_once(&self, app: AppHandle) {
        if self.started.swap(true, Ordering::Relaxed) {
            return;
        }

        let enabled = self.enabled.clone();
        thread::spawn(move || {
            info!("Type to Search listener started");
            let modifiers = Arc::new(parking_lot::Mutex::new(ModifierState::default()));

            let callback = move |event: rdev::Event| {
                update_modifiers(&modifiers, &event.event_type);
                if !enabled.load(Ordering::Relaxed) {
                    return;
                }

                let EventType::KeyPress(key) = event.event_type else {
                    return;
                };
                if is_modifier_key(key) || modifiers.lock().any_pressed() {
                    return;
                }
                if is_quikfind_visible(&app) || !is_on_desktop() {
                    return;
                }
                if let Some(ch) = event_name_to_char(event.name.as_deref()) {
                    let _ = app.emit("desktop-key", ch.to_string());
                }
            };

            if let Err(err) = listen(callback) {
                error!("Type to Search listener error: {:?}", err);
            }
        });
    }
}

#[derive(Default)]
struct ModifierState {
    ctrl: bool,
    alt: bool,
    shift: bool,
    meta: bool,
}

impl ModifierState {
    const fn any_pressed(&self) -> bool {
        self.ctrl || self.alt || self.shift || self.meta
    }
}

fn update_modifiers(modifiers: &parking_lot::Mutex<ModifierState>, event_type: &EventType) {
    let (key, pressed) = match event_type {
        EventType::KeyPress(key) => (*key, true),
        EventType::KeyRelease(key) => (*key, false),
        _ => return,
    };

    let mut state = modifiers.lock();
    match key {
        Key::ControlLeft | Key::ControlRight => state.ctrl = pressed,
        Key::Alt | Key::AltGr => state.alt = pressed,
        Key::ShiftLeft | Key::ShiftRight => state.shift = pressed,
        Key::MetaLeft | Key::MetaRight => state.meta = pressed,
        _ => {}
    }
}

fn is_quikfind_visible(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

fn event_name_to_char(name: Option<&str>) -> Option<char> {
    let name = name?;
    let mut chars = name.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if ch.is_control() {
        return None;
    }
    Some(ch)
}

fn is_modifier_key(key: Key) -> bool {
    matches!(
        key,
        Key::ControlLeft
            | Key::ControlRight
            | Key::Alt
            | Key::AltGr
            | Key::ShiftLeft
            | Key::ShiftRight
            | Key::MetaLeft
            | Key::MetaRight
    )
}
