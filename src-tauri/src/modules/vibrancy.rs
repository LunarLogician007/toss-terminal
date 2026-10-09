// TOSS Terminal: the see-through window's backdrop on macOS.
//
// Ghostty's approach (`background-blur`): a transparent window plus a plain
// background blur from the window server, with no tinted material. An
// NSVisualEffectView material (what the original used) reads as a nearly
// solid window background, which is not the see-through look this is for.
use serde::Serialize;

/// The window backdrop the platform provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backdrop {
    /// macOS window-server background blur.
    Blur,
    None,
}

/// Blur radius behind the window, as Ghostty's `background-blur = 20`.
pub const BLUR_RADIUS: i32 = 20;

pub fn backdrop_for(os: &str) -> Backdrop {
    match os {
        "macos" => Backdrop::Blur,
        _ => Backdrop::None,
    }
}

#[tauri::command]
pub fn window_backdrop_kind() -> Backdrop {
    backdrop_for(std::env::consts::OS)
}

#[tauri::command]
pub fn window_set_backdrop(window: tauri::Window, enabled: bool) -> Result<(), String> {
    set_backdrop(&window, enabled)
}

#[cfg(target_os = "macos")]
mod blur {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use std::ffi::c_void;

    // Private window-server calls, the same ones Ghostty uses for its blur.
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGSDefaultConnectionForThread() -> i32;
        fn CGSSetWindowBackgroundBlurRadius(connection: i32, window: u32, radius: i32) -> i32;
    }

    pub fn set(ns_window: *mut c_void, radius: i32) -> Result<(), String> {
        if ns_window.is_null() {
            return Err("the window has no NSWindow".into());
        }
        let window: &AnyObject = unsafe { &*(ns_window as *const AnyObject) };
        let number: isize = unsafe { msg_send![window, windowNumber] };
        let err = unsafe {
            CGSSetWindowBackgroundBlurRadius(CGSDefaultConnectionForThread(), number as u32, radius)
        };
        if err == 0 {
            Ok(())
        } else {
            Err(format!("CGSSetWindowBackgroundBlurRadius failed: {err}"))
        }
    }
}

#[cfg(target_os = "macos")]
fn set_backdrop(window: &tauri::Window, enabled: bool) -> Result<(), String> {
    let ns_window = window.ns_window().map_err(|e| e.to_string())?;
    blur::set(ns_window, if enabled { BLUR_RADIUS } else { 0 })
}

#[cfg(not(target_os = "macos"))]
fn set_backdrop(_window: &tauri::Window, _enabled: bool) -> Result<(), String> {
    Ok(())
}

/// Hide or show macOS's close, minimise and zoom buttons. With the top bar
/// hidden they would sit on the terminal's first line; the keys (Cmd+W,
/// Cmd+M, Cmd+Q) still work.
#[tauri::command]
pub fn window_set_buttons_hidden(window: tauri::Window, hidden: bool) -> Result<(), String> {
    set_buttons_hidden(&window, hidden)
}

#[cfg(target_os = "macos")]
fn set_buttons_hidden(window: &tauri::Window, hidden: bool) -> Result<(), String> {
    let ns_window = window.ns_window().map_err(|e| e.to_string())? as usize;
    window
        .run_on_main_thread(move || buttons::set_hidden(ns_window, hidden))
        .map_err(|e| e.to_string())
}

#[cfg(not(target_os = "macos"))]
fn set_buttons_hidden(_window: &tauri::Window, _hidden: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
mod buttons {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    /// NSWindowCloseButton, NSWindowMiniaturizeButton, NSWindowZoomButton.
    const KINDS: [usize; 3] = [0, 1, 2];

    pub fn set_hidden(ns_window: usize, hidden: bool) {
        if ns_window == 0 {
            return;
        }
        let window: &AnyObject = unsafe { &*(ns_window as *const AnyObject) };
        for kind in KINDS {
            let button: *mut AnyObject = unsafe { msg_send![window, standardWindowButton: kind] };
            if let Some(button) = unsafe { button.as_ref() } {
                let _: () = unsafe { msg_send![button, setHidden: hidden] };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{backdrop_for, Backdrop};

    #[test]
    fn macos_reports_blur() {
        assert_eq!(backdrop_for("macos"), Backdrop::Blur);
    }

    #[test]
    fn other_platforms_report_none() {
        assert_eq!(backdrop_for("linux"), Backdrop::None);
        assert_eq!(backdrop_for("windows"), Backdrop::None);
    }
}
