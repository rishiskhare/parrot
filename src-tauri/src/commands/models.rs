use crate::managers::model::{ModelInfo, ModelManager};
use crate::managers::tts::{TTSManager, MODEL_ID as TTS_MODEL_ID};
use crate::settings::{get_settings, write_settings};
use std::sync::Arc;
use tauri::{AppHandle, State};

#[tauri::command]
#[specta::specta]
pub async fn get_available_models(
    model_manager: State<'_, Arc<ModelManager>>,
) -> Result<Vec<ModelInfo>, String> {
    Ok(model_manager.get_available_models())
}

#[tauri::command]
#[specta::specta]
pub async fn get_model_info(
    model_manager: State<'_, Arc<ModelManager>>,
    model_id: String,
) -> Result<Option<ModelInfo>, String> {
    Ok(model_manager.get_model_info(&model_id))
}

#[tauri::command]
#[specta::specta]
pub async fn get_kokoro_voices(
    tts_manager: State<'_, Arc<TTSManager>>,
) -> Result<Vec<String>, String> {
    tts_manager
        .get_available_voices()
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn download_model(
    model_manager: State<'_, Arc<ModelManager>>,
    model_id: String,
) -> Result<(), String> {
    model_manager
        .download_model(&model_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_model(
    app_handle: AppHandle,
    model_manager: State<'_, Arc<ModelManager>>,
    tts_manager: State<'_, Arc<TTSManager>>,
    model_id: String,
) -> Result<(), String> {
    // If deleting the active model, clear the setting
    let settings = get_settings(&app_handle);
    if settings.selected_model == model_id {
        let mut settings = get_settings(&app_handle);
        settings.selected_model = String::new();
        write_settings(&app_handle, settings);
    }

    // Unload TTS model from memory if it's the one being deleted
    if model_id == TTS_MODEL_ID && tts_manager.is_model_loaded() {
        tts_manager.unload_model().map_err(|e| e.to_string())?;
    }

    model_manager
        .delete_model(&model_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn set_active_model(
    app_handle: AppHandle,
    model_manager: State<'_, Arc<ModelManager>>,
    tts_manager: State<'_, Arc<TTSManager>>,
    model_id: String,
) -> Result<(), String> {
    // Check if model exists and is available
    let model_info = model_manager
        .get_model_info(&model_id)
        .ok_or_else(|| format!("Model not found: {}", model_id))?;

    if !model_info.is_downloaded {
        return Err(format!("Model not downloaded: {}", model_id));
    }

    // Persist the selection
    let mut settings = get_settings(&app_handle);
    settings.selected_model = model_id.clone();
    write_settings(&app_handle, settings);

    // Preload the TTS model in the background
    if model_id == TTS_MODEL_ID {
        tts_manager.initiate_model_load();
    }

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn get_current_model(
    app_handle: AppHandle,
    model_manager: State<'_, Arc<ModelManager>>,
    tts_manager: State<'_, Arc<TTSManager>>,
) -> Result<String, String> {
    if let Some(runtime_model) = tts_manager.get_current_model() {
        return Ok(runtime_model);
    }

    let settings = get_settings(&app_handle);
    let selected_model = settings.selected_model;
    if selected_model.is_empty() {
        return Ok(String::new());
    }

    let model_is_downloaded = model_manager
        .get_model_info(&selected_model)
        .map(|model| model.is_downloaded)
        .unwrap_or(false);
    if !model_is_downloaded {
        return Ok(String::new());
    }

    Ok(selected_model)
}

#[tauri::command]
#[specta::specta]
pub async fn get_tts_model_status(
    tts_manager: State<'_, Arc<TTSManager>>,
) -> Result<Option<String>, String> {
    Ok(tts_manager.get_current_model())
}

#[tauri::command]
#[specta::specta]
pub async fn is_model_loading(tts_manager: State<'_, Arc<TTSManager>>) -> Result<bool, String> {
    Ok(tts_manager.is_model_loading())
}

#[tauri::command]
#[specta::specta]
pub async fn has_any_models_available(
    model_manager: State<'_, Arc<ModelManager>>,
) -> Result<bool, String> {
    let models = model_manager.get_available_models();
    Ok(models.iter().any(|m| m.is_downloaded))
}

#[tauri::command]
#[specta::specta]
pub async fn has_any_models_or_downloads(
    model_manager: State<'_, Arc<ModelManager>>,
) -> Result<bool, String> {
    let models = model_manager.get_available_models();
    // Return true if any models are downloaded OR if any downloads are in progress
    Ok(models.iter().any(|m| m.is_downloaded))
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_download(
    model_manager: State<'_, Arc<ModelManager>>,
    model_id: String,
) -> Result<(), String> {
    model_manager
        .cancel_download(&model_id)
        .map_err(|e| e.to_string())
}
