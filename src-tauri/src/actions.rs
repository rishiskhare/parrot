use crate::managers::tts::TTSManager;
use crate::utils::show_processing_overlay;
use log::{debug, info};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

const SHORTCUT_SETTLE_DELAY_MS: u64 = 40;

// Shortcut Action Trait
pub trait ShortcutAction: Send + Sync {
    fn start(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str);
    fn stop(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str);
}

// Play/Pause Action
struct PlayPauseAction;

impl ShortcutAction for PlayPauseAction {
    fn start(&self, app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        if let Some(speech) = app.try_state::<Arc<TTSManager>>() {
            if let Ok(is_paused) = speech.toggle_pause() {
                if let Some(overlay_window) = app.get_webview_window("speaking_overlay") {
                    let _ = overlay_window.emit("tts-pause-state", is_paused);
                }
            }
        }
    }

    fn stop(&self, _app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        // Nothing to do on key release
    }
}

// Test Action
struct TestAction;

impl ShortcutAction for TestAction {
    fn start(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str) {
        info!(
            "Shortcut ID '{}': Started - {} (App: {})",
            binding_id,
            shortcut_str,
            app.package_info().name
        );
    }

    fn stop(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str) {
        info!(
            "Shortcut ID '{}': Stopped - {} (App: {})",
            binding_id,
            shortcut_str,
            app.package_info().name
        );
    }
}

// Speak Action — reads selected text via macOS Accessibility API and speaks it with Kokoro TTS.
struct SpeakAction;

/// Read the currently selected text using the macOS Accessibility API.
/// Does not touch the clipboard. Returns `None` when nothing is selected or
/// accessibility is unavailable.
#[cfg(target_os = "macos")]
fn get_selected_text() -> Option<String> {
    use std::ffi::{c_char, c_void, CStr};
    use std::ptr;

    type Ptr = *mut c_void;
    const UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateSystemWide() -> Ptr;
        fn AXUIElementCopyAttributeValue(element: Ptr, attribute: Ptr, value: *mut Ptr) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: Ptr);
        fn CFStringCreateWithBytes(
            alloc: *const c_void,
            bytes: *const u8,
            num_bytes: i64,
            encoding: u32,
            is_external: bool,
        ) -> Ptr;
        fn CFStringGetLength(s: Ptr) -> i64;
        fn CFStringGetCString(s: Ptr, buf: *mut c_char, buf_size: i64, encoding: u32) -> bool;
    }

    unsafe fn cf_str(bytes: &[u8]) -> Ptr {
        CFStringCreateWithBytes(ptr::null(), bytes.as_ptr(), bytes.len() as i64, UTF8, false)
    }

    unsafe fn cf_to_string(ptr: Ptr) -> Option<String> {
        if ptr.is_null() {
            return None;
        }
        let len = CFStringGetLength(ptr);
        let buf_size = len * 4 + 1; // worst-case UTF-8 bytes + NUL
        let mut buf = vec![0u8; buf_size as usize];
        let ok = CFStringGetCString(ptr, buf.as_mut_ptr() as *mut c_char, buf_size, UTF8);
        CFRelease(ptr);
        if !ok {
            return None;
        }
        CStr::from_ptr(buf.as_ptr() as *const c_char)
            .to_str()
            .ok()
            .map(str::to_owned)
    }

    unsafe {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            return None;
        }

        let focused_attr = cf_str(b"AXFocusedUIElement");
        let mut focused: Ptr = ptr::null_mut();
        let err = AXUIElementCopyAttributeValue(system, focused_attr, &mut focused);
        CFRelease(focused_attr);
        CFRelease(system);
        if err != 0 || focused.is_null() {
            return None;
        }

        let text_attr = cf_str(b"AXSelectedText");
        let mut value: Ptr = ptr::null_mut();
        let err = AXUIElementCopyAttributeValue(focused, text_attr, &mut value);
        CFRelease(text_attr);
        CFRelease(focused);
        if err != 0 || value.is_null() {
            return None;
        }

        cf_to_string(value).filter(|s| !s.trim().is_empty())
    }
}

#[cfg(target_os = "macos")]
fn get_selected_text_with_fallback(app: &AppHandle) -> Option<String> {
    // Retry AX selection reads because some apps only expose selection once the
    // shortcut state has settled.
    for delay_ms in [0_u64, 40, 90] {
        if delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        }
        if let Some(text) = get_selected_text() {
            return Some(text);
        }
    }

    // Fallback: trigger Cmd+C and read clipboard while restoring original content.
    let clipboard = app.clipboard();
    let previous_clipboard = clipboard.read_text().ok();
    let restore_clipboard = |value: Option<String>| {
        let restore_value = value.unwrap_or_default();
        let _ = clipboard.write_text(restore_value);
    };

    // Use a sentinel so we can reliably tell whether copy actually produced text.
    let sentinel = format!(
        "__PARROT_SELECTION_PROBE_{}__",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis())
            .unwrap_or_default()
    );
    let _ = clipboard.write_text(&sentinel);

    {
        use crate::input::{send_copy_ctrl_c, EnigoState};
        let enigo_state = app.try_state::<EnigoState>()?;
        let mut enigo = enigo_state.0.lock().ok()?;
        if send_copy_ctrl_c(&mut enigo).is_err() {
            restore_clipboard(previous_clipboard);
            return None;
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(120));
    let copied_text = clipboard.read_text().ok();

    restore_clipboard(previous_clipboard);

    let copied = copied_text?.trim().to_string();
    if copied.is_empty() || copied == sentinel {
        None
    } else {
        Some(copied)
    }
}

#[cfg(not(target_os = "macos"))]
fn get_selected_text_with_fallback(app: &AppHandle) -> Option<String> {
    let clipboard = app.clipboard();
    let previous_clipboard = clipboard.read_text().ok();
    let restore_clipboard = |value: Option<String>| {
        let restore_value = value.unwrap_or_default();
        let _ = clipboard.write_text(restore_value);
    };

    // Use a sentinel so we can reliably tell whether copy actually produced text.
    let sentinel = format!(
        "__PARROT_SELECTION_PROBE_{}__",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis())
            .unwrap_or_default()
    );
    let _ = clipboard.write_text(&sentinel);

    {
        use crate::input::{send_copy_ctrl_c, EnigoState};
        let enigo_state = app.try_state::<EnigoState>()?;
        let mut enigo = enigo_state.0.lock().ok()?;
        if send_copy_ctrl_c(&mut enigo).is_err() {
            restore_clipboard(previous_clipboard);
            return None;
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(120));
    let copied_text = clipboard.read_text().ok();

    restore_clipboard(previous_clipboard);

    let copied = copied_text?.trim().to_string();
    if copied.is_empty() || copied == sentinel {
        None
    } else {
        Some(copied)
    }
}

impl ShortcutAction for SpeakAction {
    fn start(&self, app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        let speech = Arc::clone(&app.state::<Arc<TTSManager>>());
        let Some(request_id) = speech.begin_request_or_toggle_stop() else {
            return;
        };

        speech.initiate_model_load();
        let app_handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(SHORTCUT_SETTLE_DELAY_MS));

            match get_selected_text_with_fallback(&app_handle) {
                Some(text) => {
                    if !speech.is_request_active(request_id) {
                        return;
                    }
                    // Show overlay only after grabbing text — showing it before
                    // the copy causes WM_ACTIVATE to steal focus from the source
                    // app on Windows, so the injected Ctrl+C lands in the overlay
                    // instead of the text the user had selected.
                    show_processing_overlay(&app_handle);
                    debug!("Speaking {} chars via TTS", text.len());
                    speech.speak(text, request_id);
                }
                None => {
                    if !speech.stop_if_request_active(request_id) {
                        return;
                    }
                    debug!("No selected text found for TTS hotkey");
                    let _ = app_handle.emit("tts-no-selection", ());
                }
            }
        });
    }

    fn stop(&self, _app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        // Nothing to do on key release — playback continues until interrupted by another press or cancel.
    }
}

pub static ACTION_MAP: Lazy<HashMap<String, Arc<dyn ShortcutAction>>> = Lazy::new(|| {
    let mut map = HashMap::new();
    map.insert(
        "speak".to_string(),
        Arc::new(SpeakAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "play_pause".to_string(),
        Arc::new(PlayPauseAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "test".to_string(),
        Arc::new(TestAction) as Arc<dyn ShortcutAction>,
    );
    map
});
