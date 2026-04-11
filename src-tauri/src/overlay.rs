use crate::input;
use crate::settings;
use crate::settings::OverlayPosition;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

#[derive(Serialize, Clone)]
struct OverlayPayload {
    state: String,
    text: Option<String>,
}

#[cfg(not(target_os = "macos"))]
use log::debug;

#[cfg(not(target_os = "macos"))]
use tauri::WebviewWindowBuilder;

#[cfg(target_os = "macos")]
use tauri::WebviewUrl;

#[cfg(target_os = "macos")]
use tauri_nspanel::{tauri_panel, CollectionBehavior, PanelBuilder, PanelLevel};

#[cfg(target_os = "linux")]
use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
#[cfg(target_os = "linux")]
use std::env;

#[cfg(target_os = "macos")]
tauri_panel! {
    panel!(SpeakingOverlayPanel {
        config: {
            can_become_key_window: false,
            is_floating_panel: true
        }
    })
}

// Overlay width is constant across states (the CSS content transitions within it).
const OVERLAY_WIDTH: f64 = 572.0;

// Per-state heights keep the window tightly fitted to the visible content,
// matching the Handy pattern where the window IS the overlay.
// Each height = CSS content height + 10px top padding for close-button overflow.
const PROCESSING_HEIGHT: f64 = 46.0;
const SPEAKING_HEIGHT: f64 = 114.0;

#[cfg(target_os = "macos")]
const OVERLAY_TOP_OFFSET: f64 = 46.0;
#[cfg(any(target_os = "windows", target_os = "linux"))]
const OVERLAY_TOP_OFFSET: f64 = 4.0;

#[cfg(target_os = "macos")]
const OVERLAY_BOTTOM_OFFSET: f64 = 15.0;

#[cfg(any(target_os = "windows", target_os = "linux"))]
const OVERLAY_BOTTOM_OFFSET: f64 = 40.0;

/// Returns the window height for a given overlay state.
fn height_for_state(state: &str) -> f64 {
    match state {
        "speaking" => SPEAKING_HEIGHT,
        _ => PROCESSING_HEIGHT,
    }
}

#[cfg(target_os = "linux")]
fn update_gtk_layer_shell_anchors(overlay_window: &tauri::webview::WebviewWindow) {
    let window_clone = overlay_window.clone();
    let _ = overlay_window.run_on_main_thread(move || {
        // Try to get the GTK window from the Tauri webview
        if let Ok(gtk_window) = window_clone.gtk_window() {
            let settings = settings::get_settings(window_clone.app_handle());
            match settings.overlay_position {
                OverlayPosition::Top => {
                    gtk_window.set_anchor(Edge::Top, true);
                    gtk_window.set_anchor(Edge::Bottom, false);
                }
                OverlayPosition::Bottom | OverlayPosition::None => {
                    gtk_window.set_anchor(Edge::Bottom, true);
                    gtk_window.set_anchor(Edge::Top, false);
                }
            }
        }
    });
}

/// Initializes GTK layer shell for Linux overlay window
/// Returns true if layer shell was successfully initialized, false otherwise
#[cfg(target_os = "linux")]
fn init_gtk_layer_shell(overlay_window: &tauri::webview::WebviewWindow) -> bool {
    // On KDE Wayland, layer-shell init has shown protocol instability.
    // Fall back to regular always-on-top overlay behavior (as in v0.7.1).
    let is_wayland = env::var("WAYLAND_DISPLAY").is_ok()
        || env::var("XDG_SESSION_TYPE")
            .map(|v| v.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false);
    let is_kde = env::var("XDG_CURRENT_DESKTOP")
        .map(|v| v.to_uppercase().contains("KDE"))
        .unwrap_or(false)
        || env::var("KDE_SESSION_VERSION").is_ok();
    if is_wayland && is_kde {
        debug!("Skipping GTK layer shell init on KDE Wayland");
        return false;
    }

    if !gtk_layer_shell::is_supported() {
        return false;
    }

    // Try to get the GTK window from the Tauri webview
    if let Ok(gtk_window) = overlay_window.gtk_window() {
        // Initialize layer shell
        gtk_window.init_layer_shell();
        gtk_window.set_layer(Layer::Overlay);
        gtk_window.set_keyboard_mode(KeyboardMode::None);
        gtk_window.set_exclusive_zone(0);

        update_gtk_layer_shell_anchors(overlay_window);

        return true;
    }
    false
}

/// Forces a window to be topmost using Win32 API (Windows only)
/// This is more reliable than Tauri's set_always_on_top which can be overridden
#[cfg(target_os = "windows")]
fn force_overlay_topmost(overlay_window: &tauri::webview::WebviewWindow) {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    };

    // Clone because run_on_main_thread takes 'static
    let overlay_clone = overlay_window.clone();

    // Make sure the Win32 call happens on the UI thread
    let _ = overlay_clone.clone().run_on_main_thread(move || {
        if let Ok(hwnd) = overlay_clone.hwnd() {
            unsafe {
                // Force Z-order: make this window topmost without changing size/pos or stealing focus
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
        }
    });
}

fn get_monitor_with_cursor(app_handle: &AppHandle) -> Option<tauri::Monitor> {
    if let Some(mouse_location) = input::get_cursor_position(app_handle) {
        if let Ok(monitors) = app_handle.available_monitors() {
            for monitor in monitors {
                let is_within =
                    is_mouse_within_monitor(mouse_location, monitor.position(), monitor.size());
                if is_within {
                    return Some(monitor);
                }
            }
        }
    }

    app_handle.primary_monitor().ok().flatten()
}

fn is_mouse_within_monitor(
    mouse_pos: (i32, i32),
    monitor_pos: &PhysicalPosition<i32>,
    monitor_size: &PhysicalSize<u32>,
) -> bool {
    let (mouse_x, mouse_y) = mouse_pos;
    let PhysicalPosition {
        x: monitor_x,
        y: monitor_y,
    } = *monitor_pos;
    let PhysicalSize {
        width: monitor_width,
        height: monitor_height,
    } = *monitor_size;

    mouse_x >= monitor_x
        && mouse_x < (monitor_x + monitor_width as i32)
        && mouse_y >= monitor_y
        && mouse_y < (monitor_y + monitor_height as i32)
}

fn calculate_overlay_position(
    app_handle: &AppHandle,
    overlay_width: f64,
    overlay_height: f64,
) -> Option<(f64, f64)> {
    if let Some(monitor) = get_monitor_with_cursor(app_handle) {
        let scale = monitor.scale_factor();
        let settings = settings::get_settings(app_handle);

        // Use work area for horizontal centering (respects side docks).
        let work_area = monitor.work_area();
        let work_area_width = work_area.size.width as f64 / scale;
        let work_area_x = work_area.position.x as f64 / scale;
        let x = work_area_x + (work_area_width - overlay_width) / 2.0;

        let y = match settings.overlay_position {
            OverlayPosition::Top => {
                // Top: position below the menu bar using work area.
                let work_area_y = work_area.position.y as f64 / scale;
                work_area_y + OVERLAY_TOP_OFFSET
            }
            OverlayPosition::Bottom | OverlayPosition::None => {
                // Bottom: position relative to the full screen edge so the
                // overlay floats just above the screen bottom. The NSPanel
                // (macOS) renders above the dock at PanelLevel::Status, and
                // on other platforms always_on_top puts it above the taskbar.
                let screen_height = monitor.size().height as f64 / scale;
                let screen_y = monitor.position().y as f64 / scale;
                screen_y + screen_height - overlay_height - OVERLAY_BOTTOM_OFFSET
            }
        };

        return Some((x, y));
    }
    None
}

/// Resizes the overlay window and repositions it so the anchored edge
/// (bottom or top, depending on settings) stays in the correct place.
fn resize_and_reposition(
    app_handle: &AppHandle,
    overlay_window: &tauri::webview::WebviewWindow,
    height: f64,
) {
    let _ = overlay_window.set_size(tauri::Size::Logical(tauri::LogicalSize {
        width: OVERLAY_WIDTH,
        height,
    }));

    #[cfg(target_os = "linux")]
    {
        update_gtk_layer_shell_anchors(overlay_window);
    }

    if let Some((x, y)) = calculate_overlay_position(app_handle, OVERLAY_WIDTH, height) {
        let _ =
            overlay_window.set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }));
    }
}

/// Creates the speaking overlay window and keeps it hidden by default
#[cfg(not(target_os = "macos"))]
pub fn create_speaking_overlay(app_handle: &AppHandle) {
    let position = calculate_overlay_position(app_handle, OVERLAY_WIDTH, PROCESSING_HEIGHT);

    // On Linux (Wayland), monitor detection often fails, but we don't need exact coordinates
    // for Layer Shell as we use anchors. On other platforms, we require a position.
    #[cfg(not(target_os = "linux"))]
    if position.is_none() {
        debug!("Failed to determine overlay position, not creating overlay window");
        return;
    }

    let mut builder = WebviewWindowBuilder::new(
        app_handle,
        "speaking_overlay",
        tauri::WebviewUrl::App("src/overlay/index.html".into()),
    )
    .title("Speaking")
    .resizable(false)
    .focusable(false)
    .inner_size(OVERLAY_WIDTH, PROCESSING_HEIGHT)
    .shadow(false)
    .maximizable(false)
    .minimizable(false)
    .closable(false)
    .accept_first_mouse(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .transparent(true)
    .focused(false)
    .visible(false);

    if let Some((x, y)) = position {
        builder = builder.position(x, y);
    }

    match builder.build() {
        #[cfg(target_os = "linux")]
        Ok(window) => {
            if init_gtk_layer_shell(&window) {
                debug!("GTK layer shell initialized for overlay window");
            } else {
                debug!("GTK layer shell not available, falling back to regular window");
            }
            debug!("Speaking overlay window created successfully (hidden)");
        }
        #[cfg(not(target_os = "linux"))]
        Ok(_) => {
            debug!("Speaking overlay window created successfully (hidden)");
        }
        Err(e) => {
            debug!("Failed to create speaking overlay window: {}", e);
        }
    }
}

/// Creates the speaking overlay panel and keeps it hidden by default (macOS)
#[cfg(target_os = "macos")]
pub fn create_speaking_overlay(app_handle: &AppHandle) {
    if let Some((x, y)) = calculate_overlay_position(app_handle, OVERLAY_WIDTH, PROCESSING_HEIGHT) {
        // PanelBuilder creates a Tauri window then converts it to NSPanel.
        // The window remains registered, so get_webview_window() still works.
        match PanelBuilder::<_, SpeakingOverlayPanel>::new(app_handle, "speaking_overlay")
            .url(WebviewUrl::App("src/overlay/index.html".into()))
            .title("Speaking")
            .position(tauri::Position::Logical(tauri::LogicalPosition { x, y }))
            .level(PanelLevel::Status)
            .size(tauri::Size::Logical(tauri::LogicalSize {
                width: OVERLAY_WIDTH,
                height: PROCESSING_HEIGHT,
            }))
            .has_shadow(false)
            .transparent(true)
            .no_activate(true)
            .corner_radius(0.0)
            .with_window(|w| w.decorations(false).transparent(true).focusable(false))
            .collection_behavior(
                CollectionBehavior::new()
                    .can_join_all_spaces()
                    .full_screen_auxiliary(),
            )
            .build()
        {
            Ok(panel) => {
                panel.hide();
            }
            Err(e) => {
                log::error!("Failed to create speaking overlay panel: {}", e);
            }
        }
    }
}

fn show_overlay_state(app_handle: &AppHandle, state: &str, text: Option<String>) {
    // Check if overlay should be shown based on position setting
    let settings = settings::get_settings(app_handle);
    if settings.overlay_position == OverlayPosition::None {
        return;
    }

    if let Some(overlay_window) = app_handle.get_webview_window("speaking_overlay") {
        // Resize the window to fit the current state and reposition so the
        // anchored edge (bottom/top) stays at the correct screen location.
        let height = height_for_state(state);
        resize_and_reposition(app_handle, &overlay_window, height);

        let _ = overlay_window.show();

        // On Windows, aggressively re-assert "topmost" in the native Z-order after showing
        #[cfg(target_os = "windows")]
        force_overlay_topmost(&overlay_window);

        let payload = OverlayPayload {
            state: state.to_string(),
            text,
        };
        let _ = overlay_window.emit("show-overlay", payload);
    }
}

/// Shows the speaking overlay window with the spoken text.
pub fn show_speaking_overlay(app_handle: &AppHandle, text: Option<String>) {
    show_overlay_state(app_handle, "speaking", text);
}

/// Shows the processing overlay window.
pub fn show_processing_overlay(app_handle: &AppHandle) {
    show_overlay_state(app_handle, "processing", None);
}

/// Updates the overlay window position based on current settings.
/// Queries the current window size so the position matches the active state.
pub fn update_overlay_position(app_handle: &AppHandle) {
    if let Some(overlay_window) = app_handle.get_webview_window("speaking_overlay") {
        #[cfg(target_os = "linux")]
        {
            update_gtk_layer_shell_anchors(&overlay_window);
        }

        // Use the window's actual size so the position calculation stays
        // consistent with whatever state the overlay is currently in.
        let current_height = overlay_window
            .inner_size()
            .ok()
            .map(|s| {
                let scale = overlay_window.scale_factor().unwrap_or(1.0);
                s.height as f64 / scale
            })
            .unwrap_or(PROCESSING_HEIGHT);

        if let Some((x, y)) = calculate_overlay_position(app_handle, OVERLAY_WIDTH, current_height)
        {
            let _ = overlay_window
                .set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }));
        }
    }
}

/// Resizes the overlay window to fit the given content height (in logical
/// pixels, as measured by the frontend's ResizeObserver) plus the #root
/// padding. Called by the frontend whenever the rendered content size changes.
pub fn resize_overlay_to_content(app_handle: &AppHandle, content_height: f64) {
    // Add the 10px top padding that #root uses for close-button overflow.
    let window_height = content_height + 10.0;

    if let Some(overlay_window) = app_handle.get_webview_window("speaking_overlay") {
        resize_and_reposition(app_handle, &overlay_window, window_height);
    }
}

/// Hides the speaking overlay window with fade-out animation.
pub fn hide_speaking_overlay(app_handle: &AppHandle) {
    // Always hide the overlay regardless of settings - if setting was changed while speaking,
    // we still want to hide it properly
    if let Some(overlay_window) = app_handle.get_webview_window("speaking_overlay") {
        // Emit event to trigger fade-out animation
        let _ = overlay_window.emit("hide-overlay", ());
        // Hide the window after a short delay to allow animation to complete
        let window_clone = overlay_window.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            let _ = window_clone.hide();
        });
    }
}
