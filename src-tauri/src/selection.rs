use crate::settings::{self, ClipboardHandling, SelectionCaptureMethod};
use log::{debug, warn};
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

const ACCESSIBILITY_RETRY_DELAYS_MS: [u64; 3] = [0, 40, 90];
const CLIPBOARD_COPY_DELAY_MS: u64 = 120;

pub fn capture_selected_text(app: &AppHandle) -> Option<String> {
    let settings = settings::get_settings(app);

    match settings.selection_capture_method {
        SelectionCaptureMethod::Auto => {
            #[cfg(target_os = "macos")]
            {
                capture_via_accessibility()
                    .or_else(|| capture_via_clipboard(app, settings.clipboard_handling))
            }

            #[cfg(not(target_os = "macos"))]
            {
                capture_via_clipboard(app, settings.clipboard_handling)
            }
        }
        SelectionCaptureMethod::Accessibility => {
            #[cfg(target_os = "macos")]
            {
                capture_via_accessibility()
            }

            #[cfg(not(target_os = "macos"))]
            {
                warn!("Accessibility capture is not supported on this platform; falling back to clipboard capture");
                capture_via_clipboard(app, settings.clipboard_handling)
            }
        }
        SelectionCaptureMethod::Clipboard => {
            capture_via_clipboard(app, settings.clipboard_handling)
        }
    }
}

#[cfg(target_os = "macos")]
fn capture_via_accessibility() -> Option<String> {
    for delay_ms in ACCESSIBILITY_RETRY_DELAYS_MS {
        if delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        }
        if let Some(text) = get_selected_text() {
            debug!("Captured selected text via Accessibility API");
            return Some(text);
        }
    }

    None
}

#[cfg(target_os = "macos")]
fn get_selected_text() -> Option<String> {
    use std::ffi::{c_char, c_void, CStr};
    use std::ptr;

    type Ptr = *mut c_void;
    const UTF8: u32 = 0x0800_0100;

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
        fn CFStringGetMaximumSizeForEncoding(length: i64, encoding: u32) -> i64;
    }

    unsafe fn cf_str(bytes: &[u8]) -> Ptr {
        CFStringCreateWithBytes(ptr::null(), bytes.as_ptr(), bytes.len() as i64, UTF8, false)
    }

    unsafe fn cf_to_string(ptr: Ptr) -> Option<String> {
        if ptr.is_null() {
            return None;
        }
        let len = CFStringGetLength(ptr);
        let buf_size = CFStringGetMaximumSizeForEncoding(len, UTF8) + 1;
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

fn capture_via_clipboard(app: &AppHandle, handling: ClipboardHandling) -> Option<String> {
    let clipboard = app.clipboard();
    let previous_clipboard = clipboard.read_text().ok();
    let sentinel = format!(
        "__PARROT_SELECTION_PROBE_{}__",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis())
            .unwrap_or_default()
    );

    if clipboard.write_text(&sentinel).is_err() {
        warn!("Failed to prime clipboard before selection capture");
        return None;
    }

    {
        use crate::input::{send_copy_ctrl_c, EnigoState};
        let enigo_state = app.try_state::<EnigoState>()?;
        let mut enigo = enigo_state.0.lock().ok()?;
        if let Err(err) = send_copy_ctrl_c(&mut enigo) {
            debug!(
                "Failed to send copy shortcut for selection capture: {}",
                err
            );
            restore_clipboard(&clipboard, previous_clipboard.as_ref());
            return None;
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(CLIPBOARD_COPY_DELAY_MS));

    let copied_text = clipboard.read_text().ok();
    let captured = copied_text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty() && *text != sentinel)
        .map(str::to_owned);

    match handling {
        ClipboardHandling::DontModify => restore_clipboard(&clipboard, previous_clipboard.as_ref()),
        ClipboardHandling::CopyToClipboard => {
            if captured.is_none() {
                restore_clipboard(&clipboard, previous_clipboard.as_ref());
            }
        }
    }

    if captured.is_some() {
        debug!("Captured selected text via clipboard copy");
    }

    captured
}

fn restore_clipboard<R: tauri::Runtime>(
    clipboard: &tauri_plugin_clipboard_manager::Clipboard<R>,
    previous_text: Option<&String>,
) {
    match previous_text {
        Some(text) => {
            let _ = clipboard.write_text(text);
        }
        None => {
            let _ = clipboard.clear();
        }
    }
}