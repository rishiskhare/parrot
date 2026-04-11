use crate::managers::tts::TTSManager;
use crate::shortcut;
use log::info;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

// Re-export all utility modules for easy access
// pub use crate::audio_feedback::*;
pub use crate::overlay::*;
pub use crate::tray::*;

/// Centralized cancellation function that can be called from anywhere in the app.
/// Handles cancelling TTS playback and updates UI state.
pub fn cancel_current_operation(app: &AppHandle) {
    info!("Initiating operation cancellation...");

    // Unregister dynamically-managed shortcuts
    shortcut::unregister_play_pause_shortcut(app);

    // Stop any active TTS playback and maybe unload the model
    if let Some(speech_manager) = app.try_state::<Arc<TTSManager>>() {
        speech_manager.stop();
        speech_manager.maybe_unload_immediately("cancellation");
    }

    // Update tray icon and hide overlay
    change_tray_icon(app);
    hide_speaking_overlay(app);

    info!("Operation cancellation completed - returned to idle state");
}
