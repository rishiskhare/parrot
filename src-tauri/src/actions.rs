use crate::managers::tts::TTSManager;
use crate::selection::capture_selected_text;
use crate::utils::show_processing_overlay;
use log::{debug, info};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

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

impl ShortcutAction for SpeakAction {
    fn start(&self, app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        let speech = Arc::clone(&app.state::<Arc<TTSManager>>());
        let Some(request_id) = speech.begin_request_or_toggle_stop() else {
            return;
        };

        speech.initiate_model_load();
        show_processing_overlay(app);
        let app_handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(SHORTCUT_SETTLE_DELAY_MS));

            match capture_selected_text(&app_handle) {
                Some(text) => {
                    if !speech.is_request_active(request_id) {
                        return;
                    }
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
