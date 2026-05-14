use crate::platform::is_on_desktop;
use rdev::{listen, EventType, Key};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use tauri::{AppHandle, Emitter, Manager};
use tracing::{error, info};

pub fn start_desktop_listener(app: AppHandle) {
    let is_listening = Arc::new(AtomicBool::new(true));

    thread::spawn(move || {
        info!("Desktop keyboard listener started");

        let callback = move |event: rdev::Event| {
            if !is_listening.load(Ordering::Relaxed) {
                return;
            }

            if let EventType::KeyPress(key) = event.event_type {
                // Don't emit if QuikFind window is already visible (avoids double-input)
                if is_quikfind_visible(&app) {
                    return;
                }

                if is_on_desktop() {
                    if let Some(ch) = key_to_char(key) {
                        let _ = app.emit("desktop-key", ch.to_string());
                    }
                }
            }
        };

        if let Err(error) = listen(callback) {
            error!("Global keyboard listener error: {:?}", error);
        }
    });
}

fn is_quikfind_visible(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}



fn key_to_char(key: Key) -> Option<char> {
    match key {
        Key::KeyA => Some('a'),
        Key::KeyB => Some('b'),
        Key::KeyC => Some('c'),
        Key::KeyD => Some('d'),
        Key::KeyE => Some('e'),
        Key::KeyF => Some('f'),
        Key::KeyG => Some('g'),
        Key::KeyH => Some('h'),
        Key::KeyI => Some('i'),
        Key::KeyJ => Some('j'),
        Key::KeyK => Some('k'),
        Key::KeyL => Some('l'),
        Key::KeyM => Some('m'),
        Key::KeyN => Some('n'),
        Key::KeyO => Some('o'),
        Key::KeyP => Some('p'),
        Key::KeyQ => Some('q'),
        Key::KeyR => Some('r'),
        Key::KeyS => Some('s'),
        Key::KeyT => Some('t'),
        Key::KeyU => Some('u'),
        Key::KeyV => Some('v'),
        Key::KeyW => Some('w'),
        Key::KeyX => Some('x'),
        Key::KeyY => Some('y'),
        Key::KeyZ => Some('z'),
        Key::Num0 => Some('0'),
        Key::Num1 => Some('1'),
        Key::Num2 => Some('2'),
        Key::Num3 => Some('3'),
        Key::Num4 => Some('4'),
        Key::Num5 => Some('5'),
        Key::Num6 => Some('6'),
        Key::Num7 => Some('7'),
        Key::Num8 => Some('8'),
        Key::Num9 => Some('9'),
        Key::Space => Some(' '),
        Key::Minus => Some('-'),
        Key::Equal => Some('='),
        Key::Comma => Some(','),
        Key::Dot => Some('.'),
        Key::Slash => Some('/'),
        Key::SemiColon => Some(';'),
        Key::Quote => Some('\''),
        Key::BackSlash => Some('\\'),
        Key::LeftBracket => Some('['),
        Key::RightBracket => Some(']'),
        _ => None,
    }
}
