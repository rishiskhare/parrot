pub mod audio;
pub mod history;
pub mod models;

use crate::managers::history::HistoryManager;
use crate::managers::model::ModelManager;
use crate::managers::tts::{TTSManager, MODEL_ID as TTS_MODEL_ID};
use crate::settings::{get_settings, write_settings, AppSettings, LogLevel};
use crate::utils::cancel_current_operation;
use serde::Serialize;
use specta::Type;
use std::fs;
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
#[specta::specta]
pub fn cancel_operation(app: AppHandle) {
    cancel_current_operation(&app);
}

#[tauri::command]
#[specta::specta]
pub fn get_app_dir_path(app: AppHandle) -> Result<String, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {}", e))?;

    Ok(app_data_dir.to_string_lossy().to_string())
}

#[tauri::command]
#[specta::specta]
pub fn get_app_settings(app: AppHandle) -> Result<AppSettings, String> {
    Ok(get_settings(&app))
}

#[tauri::command]
#[specta::specta]
pub fn get_default_settings() -> Result<AppSettings, String> {
    Ok(crate::settings::get_default_settings())
}

#[tauri::command]
#[specta::specta]
pub fn get_log_dir_path(app: AppHandle) -> Result<String, String> {
    let log_dir = app
        .path()
        .app_log_dir()
        .map_err(|e| format!("Failed to get log directory: {}", e))?;

    Ok(log_dir.to_string_lossy().to_string())
}

#[specta::specta]
#[tauri::command]
pub fn set_log_level(app: AppHandle, level: LogLevel) -> Result<(), String> {
    let tauri_log_level: tauri_plugin_log::LogLevel = level.into();
    let log_level: log::Level = tauri_log_level.into();
    // Update the file log level atomic so the filter picks up the new level
    crate::FILE_LOG_LEVEL.store(
        log_level.to_level_filter() as u8,
        std::sync::atomic::Ordering::Relaxed,
    );

    let mut settings = get_settings(&app);
    settings.log_level = level;
    write_settings(&app, settings);

    Ok(())
}

#[specta::specta]
#[tauri::command]
pub fn open_history_folder(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
) -> Result<(), String> {
    let audio_dir = history_manager.get_audio_dir_path();
    if !audio_dir.exists() {
        fs::create_dir_all(&audio_dir)
            .map_err(|e| format!("Failed to create audio directory: {}", e))?;
    }

    let path = audio_dir.to_string_lossy().as_ref().to_string();
    app.opener()
        .open_path(path, None::<String>)
        .map_err(|e| format!("Failed to open history folder: {}", e))?;

    Ok(())
}

#[specta::specta]
#[tauri::command]
pub fn open_log_dir(app: AppHandle) -> Result<(), String> {
    let log_dir = app
        .path()
        .app_log_dir()
        .map_err(|e| format!("Failed to get log directory: {}", e))?;

    let path = log_dir.to_string_lossy().as_ref().to_string();
    app.opener()
        .open_path(path, None::<String>)
        .map_err(|e| format!("Failed to open log directory: {}", e))?;

    Ok(())
}

#[specta::specta]
#[tauri::command]
pub fn open_app_data_dir(app: AppHandle) -> Result<(), String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {}", e))?;

    let path = app_data_dir.to_string_lossy().as_ref().to_string();
    app.opener()
        .open_path(path, None::<String>)
        .map_err(|e| format!("Failed to open app data directory: {}", e))?;

    Ok(())
}

/// Try to initialize Enigo (keyboard/mouse simulation).
/// On macOS, this will return an error if accessibility permissions are not granted.
#[specta::specta]
#[tauri::command]
pub fn initialize_enigo(app: AppHandle) -> Result<(), String> {
    use crate::input::EnigoState;

    // Check if already initialized
    if app.try_state::<EnigoState>().is_some() {
        log::debug!("Enigo already initialized");
        return Ok(());
    }

    // Try to initialize
    match EnigoState::new() {
        Ok(enigo_state) => {
            app.manage(enigo_state);
            log::info!("Enigo initialized successfully after permission grant");
            Ok(())
        }
        Err(e) => {
            if cfg!(target_os = "macos") {
                log::warn!(
                    "Failed to initialize Enigo: {} (accessibility permissions may not be granted)",
                    e
                );
            } else {
                log::warn!("Failed to initialize Enigo: {}", e);
            }
            Err(format!("Failed to initialize input system: {}", e))
        }
    }
}

/// Marker state to track if shortcuts have been initialized.
pub struct ShortcutsInitialized;

/// Initialize keyboard shortcuts.
/// On macOS, this should be called after accessibility permissions are granted.
/// This is idempotent - calling it multiple times is safe.
#[specta::specta]
#[tauri::command]
pub fn initialize_shortcuts(app: AppHandle) -> Result<(), String> {
    // Check if already initialized
    if app.try_state::<ShortcutsInitialized>().is_some() {
        log::debug!("Shortcuts already initialized");
        return Ok(());
    }

    // Initialize shortcuts
    crate::shortcut::init_shortcuts(&app);

    // Mark as initialized
    app.manage(ShortcutsInitialized);

    log::info!("Shortcuts initialized successfully");
    Ok(())
}

#[derive(Serialize, Type)]
pub struct ModelStatus {
    pub model_id: String,
    pub model_name: String,
    pub model_description: String,
    pub accuracy_score: f32,
    pub speed_score: f32,
    pub is_recommended: bool,
    pub model_dir: String,
    pub model_files_present: bool,
    pub model_loaded: bool,
}

#[specta::specta]
#[tauri::command]
pub fn get_model_status(
    app: AppHandle,
    model_manager: State<'_, Arc<ModelManager>>,
    tts_manager: State<'_, Arc<TTSManager>>,
) -> Result<ModelStatus, String> {
    let model_info = model_manager
        .get_model_info(TTS_MODEL_ID)
        .ok_or_else(|| "TTS model not found".to_string())?;

    let app_data_model_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to resolve app data directory: {}", e))?
        .join("models")
        .join(&model_info.filename);
    let bundled_model_dir = app
        .path()
        .resolve(
            format!("models/{}", model_info.filename),
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|e| format!("Failed to resolve bundled model directory: {}", e))?;

    let model_files_present = app_data_model_dir.is_dir() || bundled_model_dir.is_dir();
    let resolved_model_dir = if app_data_model_dir.is_dir() {
        app_data_model_dir
    } else if bundled_model_dir.is_dir() {
        bundled_model_dir
    } else {
        // Prefer app-data path for setup guidance.
        app_data_model_dir
    };

    Ok(ModelStatus {
        model_id: model_info.id,
        model_name: model_info.name,
        model_description: model_info.description,
        accuracy_score: model_info.accuracy_score,
        speed_score: model_info.speed_score,
        is_recommended: model_info.is_recommended,
        model_dir: resolved_model_dir.to_string_lossy().to_string(),
        model_files_present,
        model_loaded: tts_manager.is_model_loaded(),
    })
}

#[specta::specta]
#[tauri::command]
pub fn preload_tts_model(tts_manager: State<'_, Arc<TTSManager>>) {
    tts_manager.initiate_model_load();
}

#[specta::specta]
#[tauri::command]
pub fn toggle_tts_pause(tts_manager: State<'_, Arc<TTSManager>>) -> Result<bool, String> {
    tts_manager
        .toggle_pause()
        .map_err(|e| format!("Failed to toggle TTS pause: {}", e))
}
