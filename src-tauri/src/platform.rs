#[cfg(target_os = "windows")]
pub(crate) fn get_all_windows_drives() -> Vec<String> {
    let drives: Vec<String> = (b'A'..=b'Z')
        .map(|letter| format!("{}:\\", letter as char))
        .filter(|drive| std::fs::metadata(drive).is_ok())
        .collect();

    if drives.is_empty() {
        vec!["C:\\".to_string()]
    } else {
        drives
    }
}

#[cfg(windows)]
#[allow(clippy::cast_sign_loss)]
pub(crate) fn is_on_desktop() -> bool {
    unsafe {
        let hwnd = winapi::um::winuser::GetForegroundWindow();
        if hwnd.is_null() {
            return false;
        }

        let mut class_name = [0u16; 256];
        let len = winapi::um::winuser::GetClassNameW(hwnd, class_name.as_mut_ptr(), 256);
        if len == 0 {
            return false;
        }

        let class_name = String::from_utf16_lossy(&class_name[..len as usize]);
        class_name == "Progman" || class_name == "WorkerW"
    }
}

#[cfg(not(windows))]
pub(crate) fn is_on_desktop() -> bool {
    false
}
