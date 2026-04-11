use crate::managers::history::{HistoryEntry, HistoryManager};
use crate::managers::tts::TTSManager;
use crate::settings;
use crate::tray_i18n::get_tray_translations;
use log::{error, info, warn};
use std::sync::Arc;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIcon;
use tauri::{AppHandle, Manager, Theme};
use tauri_plugin_clipboard_manager::ClipboardExt;

#[derive(Clone, Debug, PartialEq)]
pub enum AppTheme {
    Dark,
    Light,
    Colored, // Pink/colored theme for Linux
}

/// Gets the current app theme, with Linux defaulting to Colored theme
pub fn get_current_theme(app: &AppHandle) -> AppTheme {
    if cfg!(target_os = "linux") {
        // On Linux, always use the colored theme
        AppTheme::Colored
    } else {
        // On other platforms, map system theme to our app theme
        if let Some(main_window) = app.get_webview_window("main") {
            match main_window.theme().unwrap_or(Theme::Dark) {
                Theme::Light => AppTheme::Light,
                Theme::Dark => AppTheme::Dark,
                _ => AppTheme::Dark, // Default fallback
            }
        } else {
            AppTheme::Dark
        }
    }
}

/// Gets the appropriate icon path for the given theme and state
pub fn get_icon_path(theme: AppTheme) -> &'static str {
    match theme {
        AppTheme::Dark => "resources/tray_idle.png",
        AppTheme::Light => "resources/tray_idle_dark.png",
        AppTheme::Colored => "resources/parrot.png",
    }
}

pub fn change_tray_icon(app: &AppHandle) {
    let tray = app.state::<TrayIcon>();
    let theme = get_current_theme(app);

    let icon_path = get_icon_path(theme);

    let _ = tray.set_icon(Some(
        Image::from_path(
            app.path()
                .resolve(icon_path, tauri::path::BaseDirectory::Resource)
                .expect("failed to resolve"),
        )
        .expect("failed to set icon"),
    ));

    update_tray_menu(app, None);
}

pub fn update_tray_menu(app: &AppHandle, locale: Option<&str>) {
    let settings = settings::get_settings(app);

    let locale = locale.unwrap_or(&settings.app_language);
    let strings = get_tray_translations(Some(locale.to_string()));

    // Platform-specific accelerators
    #[cfg(target_os = "macos")]
    let (settings_accelerator, quit_accelerator) = (Some("Cmd+,"), Some("Cmd+Q"));
    #[cfg(not(target_os = "macos"))]
    let (settings_accelerator, quit_accelerator) = (Some("Ctrl+,"), Some("Ctrl+Q"));

    // Create common menu items
    let version_label = if cfg!(debug_assertions) {
        format!("Parrot v{} (Dev)", env!("CARGO_PKG_VERSION"))
    } else {
        format!("Parrot v{}", env!("CARGO_PKG_VERSION"))
    };
    let version_i = MenuItem::with_id(app, "version", &version_label, false, None::<&str>)
        .expect("failed to create version item");
    let settings_i = MenuItem::with_id(
        app,
        "settings",
        &strings.settings,
        true,
        settings_accelerator,
    )
    .expect("failed to create settings item");
    let check_updates_i = MenuItem::with_id(
        app,
        "check_updates",
        &strings.check_updates,
        settings.update_checks_enabled,
        None::<&str>,
    )
    .expect("failed to create check updates item");
    let copy_last_text_i = MenuItem::with_id(
        app,
        "copy_last_text",
        &strings.copy_last_text,
        true,
        None::<&str>,
    )
    .expect("failed to create copy last transcript item");
    let model_loaded = app.state::<Arc<TTSManager>>().is_model_loaded();
    let unload_model_i = MenuItem::with_id(
        app,
        "unload_model",
        &strings.unload_model,
        model_loaded,
        None::<&str>,
    )
    .expect("failed to create unload model item");
    let quit_i = MenuItem::with_id(app, "quit", &strings.quit, true, quit_accelerator)
        .expect("failed to create quit item");
    let separator = || PredefinedMenuItem::separator(app).expect("failed to create separator");

    let menu = Menu::with_items(
        app,
        &[
            &version_i,
            &separator(),
            &copy_last_text_i,
            &unload_model_i,
            &separator(),
            &settings_i,
            &check_updates_i,
            &separator(),
            &quit_i,
        ],
    )
    .expect("failed to create menu");

    let tray = app.state::<TrayIcon>();
    let _ = tray.set_menu(Some(menu));
    let _ = tray.set_icon_as_template(true);
}

fn last_transcript_text(entry: &HistoryEntry) -> &str {
    &entry.transcription_text
}

pub fn set_tray_visibility(app: &AppHandle, visible: bool) {
    let tray = app.state::<TrayIcon>();
    if let Err(e) = tray.set_visible(visible) {
        error!("Failed to set tray visibility: {}", e);
    } else {
        info!("Tray visibility set to: {}", visible);
    }
}

pub fn copy_last_text(app: &AppHandle) {
    let history_manager = app.state::<Arc<HistoryManager>>();
    let entry = match history_manager.get_latest_entry() {
        Ok(Some(entry)) => entry,
        Ok(None) => {
            warn!("No transcription history entries available for tray copy.");
            return;
        }
        Err(err) => {
            error!("Failed to fetch last transcription entry: {}", err);
            return;
        }
    };

    if let Err(err) = app.clipboard().write_text(last_transcript_text(&entry)) {
        error!("Failed to copy last transcript to clipboard: {}", err);
        return;
    }

    info!("Copied last transcript to clipboard via tray.");
}

#[cfg(test)]
mod tests {
    use super::last_transcript_text;
    use crate::managers::history::HistoryEntry;

    fn build_entry(transcription: &str) -> HistoryEntry {
        HistoryEntry {
            id: 1,
            file_name: "parrot-1.wav".to_string(),
            timestamp: 0,
            saved: false,
            title: "Recording".to_string(),
            transcription_text: transcription.to_string(),
        }
    }

    #[test]
    fn returns_transcription_text() {
        let entry = build_entry("raw");
        assert_eq!(last_transcript_text(&entry), "raw");
    }
}
