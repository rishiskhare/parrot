use crate::settings::{get_settings, write_settings};
use anyhow::Result;
use flate2::read::GzDecoder;
use futures_util::StreamExt;
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::model_families;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tar::Archive;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub enum EngineType {
    Kokoro,
    Piper,
}

const HF_BASE_URL: &str = "https://huggingface.co";
/// Models the user added from Hugging Face by repo id.
const HF_MODELS_FILE: &str = "huggingface-models.json";

/// A single file that must be downloaded as part of a multi-file model.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ModelComponent {
    /// Download URL for this file.
    pub url: String,
    /// Filename within the model directory once downloaded.
    pub filename: String,
    /// Approximate file size in MB (used for progress display).
    pub size_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub filename: String,
    pub url: Option<String>,
    pub size_mb: u64,
    pub is_downloaded: bool,
    pub is_downloading: bool,
    pub partial_size: u64,
    pub is_directory: bool,
    pub engine_type: EngineType,
    pub accuracy_score: f32,  // 0.0 to 1.0, higher is more accurate
    pub speed_score: f32,     // 0.0 to 1.0, higher is faster
    pub is_recommended: bool, // Whether this is the recommended model for new users
    pub supported_languages: Vec<String>, // Languages this model supports
    pub is_custom: bool,      // Whether this is a user-provided custom model
    /// For models that consist of multiple individual files rather than a single
    /// archive. When non-empty, `url` is ignored and each component is downloaded
    /// separately into the model directory.
    #[serde(default)]
    pub components: Vec<ModelComponent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DownloadProgress {
    pub model_id: String,
    pub downloaded: u64,
    pub total: u64,
    pub percentage: f64,
}

pub struct ModelManager {
    app_handle: AppHandle,
    models_dir: PathBuf,
    available_models: Mutex<HashMap<String, ModelInfo>>,
    cancel_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    extracting_models: Arc<Mutex<HashSet<String>>>,
}

impl ModelManager {
    pub fn new(app_handle: &AppHandle) -> Result<Self> {
        // Create models directory in app data
        let models_dir = app_handle
            .path()
            .app_data_dir()
            .map_err(|e| anyhow::anyhow!("Failed to get app data dir: {}", e))?
            .join("models");

        if !models_dir.exists() {
            fs::create_dir_all(&models_dir)?;
        }

        let mut available_models = HashMap::new();

        // Kokoro TTS — two component files downloaded into models/kokoro/
        available_models.insert(
            "kokoro".to_string(),
            ModelInfo {
                id: "kokoro".to_string(),
                name: "Kokoro-82M".to_string(),
                description: "Fast and accurate".to_string(),
                filename: "kokoro".to_string(), // directory name
                url: None,
                size_mb: 115, // 88 MB ONNX + 27 MB voices
                is_downloaded: false,
                is_downloading: false,
                partial_size: 0,
                is_directory: true,
                engine_type: EngineType::Kokoro,
                accuracy_score: 0.80,
                speed_score: 0.85,
                is_recommended: true,
                supported_languages: kokoro_languages(),
                is_custom: false,
                components: vec![
                    ModelComponent {
                        url: "https://github.com/taylorchu/kokoro-onnx/releases/download/v0.2.0/kokoro-quant-convinteger.onnx".to_string(),
                        filename: "kokoro-quant-convinteger.onnx".to_string(),
                        size_mb: 88,
                    },
                    ModelComponent {
                        url: "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin".to_string(),
                        filename: "voices-v1.0.bin".to_string(),
                        size_mb: 27,
                    },
                ],
            },
        );

        for model in load_cached_huggingface_models(&models_dir) {
            available_models.entry(model.id.clone()).or_insert(model);
        }

        let manager = Self {
            app_handle: app_handle.clone(),
            models_dir,
            available_models: Mutex::new(available_models),
            cancel_flags: Arc::new(Mutex::new(HashMap::new())),
            extracting_models: Arc::new(Mutex::new(HashSet::new())),
        };

        // Migrate any bundled models to user directory
        manager.migrate_bundled_models()?;

        // Check which models are already downloaded
        manager.update_download_status()?;

        // Auto-select a model if none is currently selected
        manager.auto_select_model_if_needed()?;

        Ok(manager)
    }

    pub fn get_available_models(&self) -> Vec<ModelInfo> {
        let models = self.available_models.lock().unwrap();
        models.values().cloned().collect()
    }

    pub fn model_dir(&self, model: &ModelInfo) -> PathBuf {
        self.models_dir.join(&model.filename)
    }

    fn insert_models(&self, models: Vec<ModelInfo>) {
        let mut available = self.available_models.lock().unwrap();
        for model in models {
            available.entry(model.id.clone()).or_insert(model);
        }
    }

    /// Add every supported model in a Hugging Face repo (`owner/name` or URL).
    pub async fn add_huggingface_model(&self, input: &str) -> Result<Vec<ModelInfo>> {
        let repo = parse_hf_repo(input)?;
        let url = format!("{HF_BASE_URL}/api/models/{repo}?blobs=true");
        let response = reqwest::get(&url).await?;
        if !response.status().is_success() {
            anyhow::bail!(
                "Couldn't open {repo} on Hugging Face ({}). Private and gated models aren't supported.",
                response.status()
            );
        }
        let info: HfRepoInfo = serde_json::from_str(&response.text().await?)?;
        let ids: Vec<String> = {
            let models = models_from_hf_repo(&repo, &info.siblings)?;
            let ids = models.iter().map(|m| m.id.clone()).collect();
            self.insert_models(models);
            ids
        };
        self.save_huggingface_models()?;
        self.update_download_status()?;
        Ok(ids
            .iter()
            .filter_map(|id| self.get_model_info(id))
            .collect())
    }

    fn save_huggingface_models(&self) -> Result<()> {
        let mut added: Vec<ModelInfo> = self
            .available_models
            .lock()
            .unwrap()
            .values()
            .filter(|m| m.id.starts_with("hf-"))
            .cloned()
            .collect();
        added.sort_by(|a, b| a.id.cmp(&b.id));
        fs::write(
            self.models_dir.join(HF_MODELS_FILE),
            serde_json::to_string_pretty(&added)?,
        )?;
        Ok(())
    }

    pub fn get_model_info(&self, model_id: &str) -> Option<ModelInfo> {
        let models = self.available_models.lock().unwrap();
        models.get(model_id).cloned()
    }

    fn migrate_bundled_models(&self) -> Result<()> {
        // No bundled models to migrate for TTS-only
        Ok(())
    }

    fn update_download_status(&self) -> Result<()> {
        let mut models = self.available_models.lock().unwrap();

        for model in models.values_mut() {
            if !model.components.is_empty() {
                // Multi-component model (e.g. Kokoro): ready when all component files exist.
                let model_dir = self.models_dir.join(&model.filename);
                model.is_downloaded = model
                    .components
                    .iter()
                    .all(|c| model_dir.join(&c.filename).exists());
                model.is_downloading = false;
                // Report combined partial size for any in-progress component downloads.
                model.partial_size = model
                    .components
                    .iter()
                    .map(|c| {
                        model_dir
                            .join(format!("{}.partial", &c.filename))
                            .metadata()
                            .map(|m| m.len())
                            .unwrap_or(0)
                    })
                    .sum();
            } else if model.is_directory {
                // For directory-based models, check if the directory exists
                let model_path = self.models_dir.join(&model.filename);
                let partial_path = self.models_dir.join(format!("{}.partial", &model.filename));
                let extracting_path = self
                    .models_dir
                    .join(format!("{}.extracting", &model.filename));

                // Clean up any leftover .extracting directories from interrupted extractions
                // But only if this model is NOT currently being extracted
                let is_currently_extracting = {
                    let extracting = self.extracting_models.lock().unwrap();
                    extracting.contains(&model.id)
                };
                if extracting_path.exists() && !is_currently_extracting {
                    warn!("Cleaning up interrupted extraction for model: {}", model.id);
                    let _ = fs::remove_dir_all(&extracting_path);
                }

                model.is_downloaded = model_path.exists() && model_path.is_dir();
                model.is_downloading = false;

                // Get partial file size if it exists (for the .tar.gz being downloaded)
                if partial_path.exists() {
                    model.partial_size = partial_path.metadata().map(|m| m.len()).unwrap_or(0);
                } else {
                    model.partial_size = 0;
                }
            } else {
                // For file-based models (existing logic)
                let model_path = self.models_dir.join(&model.filename);
                let partial_path = self.models_dir.join(format!("{}.partial", &model.filename));

                model.is_downloaded = model_path.exists();
                model.is_downloading = false;

                // Get partial file size if it exists
                if partial_path.exists() {
                    model.partial_size = partial_path.metadata().map(|m| m.len()).unwrap_or(0);
                } else {
                    model.partial_size = 0;
                }
            }
        }

        Ok(())
    }

    fn auto_select_model_if_needed(&self) -> Result<()> {
        let mut settings = get_settings(&self.app_handle);

        // Clear stale selection: selected model is set but doesn't exist
        // in available_models (e.g. deleted custom model file)
        if !settings.selected_model.is_empty() {
            let models = self.available_models.lock().unwrap();
            let exists = models.contains_key(&settings.selected_model);
            drop(models);

            if !exists {
                info!(
                    "Selected model '{}' not found in available models, clearing selection",
                    settings.selected_model
                );
                settings.selected_model = String::new();
                write_settings(&self.app_handle, settings.clone());
            }
        }

        // If no model is selected, pick the first downloaded one
        if settings.selected_model.is_empty() {
            // Find the first available (downloaded) model
            let models = self.available_models.lock().unwrap();
            if let Some(available_model) = models.values().find(|model| model.is_downloaded) {
                info!(
                    "Auto-selecting model: {} ({})",
                    available_model.id, available_model.name
                );

                // Update settings with the selected model
                let mut updated_settings = settings;
                updated_settings.selected_model = available_model.id.clone();
                write_settings(&self.app_handle, updated_settings);

                info!("Successfully auto-selected model: {}", available_model.id);
            }
        }

        Ok(())
    }

    pub async fn download_model(&self, model_id: &str) -> Result<()> {
        let model_info = {
            let models = self.available_models.lock().unwrap();
            models.get(model_id).cloned()
        };

        let model_info =
            model_info.ok_or_else(|| anyhow::anyhow!("Model not found: {}", model_id))?;

        // Multi-component model (e.g. Kokoro): download each file individually into a
        // subdirectory rather than fetching a single archive.
        if !model_info.components.is_empty() {
            let model_dir = self.models_dir.join(&model_info.filename);
            fs::create_dir_all(&model_dir)?;

            // Mark as downloading
            {
                let mut models = self.available_models.lock().unwrap();
                if let Some(model) = models.get_mut(model_id) {
                    model.is_downloading = true;
                }
            }

            // Single cancellation flag shared across all component downloads
            let cancel_flag = Arc::new(AtomicBool::new(false));
            {
                let mut flags = self.cancel_flags.lock().unwrap();
                flags.insert(model_id.to_string(), cancel_flag.clone());
            }

            let client = reqwest::Client::new();
            // Estimate total bytes from declared sizes; used for progress display.
            let total_size_estimate: u64 = model_info
                .components
                .iter()
                .map(|c| c.size_mb * 1024 * 1024)
                .sum();
            // Accumulates bytes from already-completed components.
            let mut prior_bytes: u64 = 0;

            for component in &model_info.components {
                let final_path = model_dir.join(&component.filename);
                let partial_path = model_dir.join(format!("{}.partial", &component.filename));

                // Skip components that are already fully downloaded.
                if final_path.exists() {
                    prior_bytes += final_path
                        .metadata()
                        .map(|m| m.len())
                        .unwrap_or(component.size_mb * 1024 * 1024);
                    continue;
                }

                // Resume from partial file if one exists.
                let mut component_resume: u64 = if partial_path.exists() {
                    let size = partial_path.metadata()?.len();
                    info!(
                        "Resuming component {} from byte {}",
                        component.filename, size
                    );
                    size
                } else {
                    info!(
                        "Downloading component {} from {}",
                        component.filename, component.url
                    );
                    0
                };

                let mut request = client.get(&component.url);
                if component_resume > 0 {
                    request = request.header("Range", format!("bytes={}-", component_resume));
                }
                let mut response = request.send().await?;

                // If server ignored our Range request and returned 200, restart fresh.
                if component_resume > 0 && response.status() == reqwest::StatusCode::OK {
                    warn!(
                        "Server doesn't support range requests for {}, restarting",
                        component.filename
                    );
                    drop(response);
                    let _ = fs::remove_file(&partial_path);
                    component_resume = 0;
                    response = client.get(&component.url).send().await?;
                }

                if !response.status().is_success()
                    && response.status() != reqwest::StatusCode::PARTIAL_CONTENT
                {
                    {
                        let mut models = self.available_models.lock().unwrap();
                        if let Some(model) = models.get_mut(model_id) {
                            model.is_downloading = false;
                        }
                    }
                    {
                        let mut flags = self.cancel_flags.lock().unwrap();
                        flags.remove(model_id);
                    }
                    return Err(anyhow::anyhow!(
                        "Failed to download {}: HTTP {}",
                        component.filename,
                        response.status()
                    ));
                }

                // Expected total bytes for this component (partial content + already downloaded).
                let component_total: u64 = component_resume
                    + response
                        .content_length()
                        .unwrap_or(component.size_mb * 1024 * 1024);
                let mut component_downloaded: u64 = component_resume;
                let mut stream = response.bytes_stream();

                let mut file = if component_resume > 0 {
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&partial_path)?
                } else {
                    std::fs::File::create(&partial_path)?
                };

                // Emit initial progress for this component
                let _ = self.app_handle.emit(
                    "model-download-progress",
                    &DownloadProgress {
                        model_id: model_id.to_string(),
                        downloaded: prior_bytes + component_downloaded,
                        total: total_size_estimate,
                        percentage: if total_size_estimate > 0 {
                            ((prior_bytes + component_downloaded) as f64
                                / total_size_estimate as f64)
                                * 100.0
                        } else {
                            0.0
                        },
                    },
                );

                let mut last_emit = Instant::now();
                let throttle_duration = Duration::from_millis(100);

                while let Some(chunk) = stream.next().await {
                    if cancel_flag.load(Ordering::Relaxed) {
                        drop(file);
                        info!("Download cancelled for: {}", model_id);
                        {
                            let mut models = self.available_models.lock().unwrap();
                            if let Some(model) = models.get_mut(model_id) {
                                model.is_downloading = false;
                            }
                        }
                        {
                            let mut flags = self.cancel_flags.lock().unwrap();
                            flags.remove(model_id);
                        }
                        return Ok(());
                    }

                    let chunk = chunk?;
                    file.write_all(&chunk)?;
                    component_downloaded += chunk.len() as u64;

                    if last_emit.elapsed() >= throttle_duration {
                        let global_downloaded = prior_bytes + component_downloaded;
                        let _ = self.app_handle.emit(
                            "model-download-progress",
                            &DownloadProgress {
                                model_id: model_id.to_string(),
                                downloaded: global_downloaded,
                                total: total_size_estimate,
                                percentage: if total_size_estimate > 0 {
                                    (global_downloaded as f64 / total_size_estimate as f64) * 100.0
                                } else {
                                    0.0
                                },
                            },
                        );
                        last_emit = Instant::now();
                    }
                }

                file.flush()?;
                drop(file);

                // Verify the file is complete.
                if component_total > 0 {
                    let actual = partial_path.metadata()?.len();
                    if actual != component_total {
                        let _ = fs::remove_file(&partial_path);
                        {
                            let mut models = self.available_models.lock().unwrap();
                            if let Some(model) = models.get_mut(model_id) {
                                model.is_downloading = false;
                            }
                        }
                        {
                            let mut flags = self.cancel_flags.lock().unwrap();
                            flags.remove(model_id);
                        }
                        return Err(anyhow::anyhow!(
                            "Download of {} incomplete: expected {} bytes, got {}",
                            component.filename,
                            component_total,
                            actual
                        ));
                    }
                }

                // Atomically promote the partial file to its final name.
                fs::rename(&partial_path, &final_path)?;
                prior_bytes += component_downloaded;
                info!("Downloaded component: {}", component.filename);
            }

            // Emit 100% progress
            let _ = self.app_handle.emit(
                "model-download-progress",
                &DownloadProgress {
                    model_id: model_id.to_string(),
                    downloaded: prior_bytes,
                    total: prior_bytes,
                    percentage: 100.0,
                },
            );

            // Mark as complete
            {
                let mut models = self.available_models.lock().unwrap();
                if let Some(model) = models.get_mut(model_id) {
                    model.is_downloading = false;
                    model.is_downloaded = true;
                    model.partial_size = 0;
                }
            }
            {
                let mut flags = self.cancel_flags.lock().unwrap();
                flags.remove(model_id);
            }

            let _ = self.app_handle.emit("model-download-complete", model_id);
            info!(
                "Successfully downloaded all components for model: {}",
                model_id
            );
            return Ok(());
        }

        let url = model_info
            .url
            .ok_or_else(|| anyhow::anyhow!("No download URL for model"))?;
        let model_path = self.models_dir.join(&model_info.filename);
        let partial_path = self
            .models_dir
            .join(format!("{}.partial", &model_info.filename));

        // Don't download if complete version already exists
        if model_path.exists() {
            // Clean up any partial file that might exist
            if partial_path.exists() {
                let _ = fs::remove_file(&partial_path);
            }
            self.update_download_status()?;
            return Ok(());
        }

        // Check if we have a partial download to resume
        let mut resume_from = if partial_path.exists() {
            let size = partial_path.metadata()?.len();
            info!("Resuming download of model {} from byte {}", model_id, size);
            size
        } else {
            info!("Starting fresh download of model {} from {}", model_id, url);
            0
        };

        // Mark as downloading
        {
            let mut models = self.available_models.lock().unwrap();
            if let Some(model) = models.get_mut(model_id) {
                model.is_downloading = true;
            }
        }

        // Create cancellation flag for this download
        let cancel_flag = Arc::new(AtomicBool::new(false));
        {
            let mut flags = self.cancel_flags.lock().unwrap();
            flags.insert(model_id.to_string(), cancel_flag.clone());
        }

        // Create HTTP client with range request for resuming
        let client = reqwest::Client::new();
        let mut request = client.get(&url);

        if resume_from > 0 {
            request = request.header("Range", format!("bytes={}-", resume_from));
        }

        let mut response = request.send().await?;

        // If we tried to resume but server returned 200 (not 206 Partial Content),
        // the server doesn't support range requests. Delete partial file and restart
        // fresh to avoid file corruption (appending full file to partial).
        if resume_from > 0 && response.status() == reqwest::StatusCode::OK {
            warn!(
                "Server doesn't support range requests for model {}, restarting download",
                model_id
            );
            drop(response);
            let _ = fs::remove_file(&partial_path);

            // Reset resume_from since we're starting fresh
            resume_from = 0;

            // Restart download without range header
            response = client.get(&url).send().await?;
        }

        // Check for success or partial content status
        if !response.status().is_success()
            && response.status() != reqwest::StatusCode::PARTIAL_CONTENT
        {
            // Mark as not downloading on error
            {
                let mut models = self.available_models.lock().unwrap();
                if let Some(model) = models.get_mut(model_id) {
                    model.is_downloading = false;
                }
            }
            return Err(anyhow::anyhow!(
                "Failed to download model: HTTP {}",
                response.status()
            ));
        }

        let total_size = if resume_from > 0 {
            // For resumed downloads, add the resume point to content length
            resume_from + response.content_length().unwrap_or(0)
        } else {
            response.content_length().unwrap_or(0)
        };

        let mut downloaded = resume_from;
        let mut stream = response.bytes_stream();

        // Open file for appending if resuming, or create new if starting fresh
        let mut file = if resume_from > 0 {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&partial_path)?
        } else {
            std::fs::File::create(&partial_path)?
        };

        // Emit initial progress
        let initial_progress = DownloadProgress {
            model_id: model_id.to_string(),
            downloaded,
            total: total_size,
            percentage: if total_size > 0 {
                (downloaded as f64 / total_size as f64) * 100.0
            } else {
                0.0
            },
        };
        let _ = self
            .app_handle
            .emit("model-download-progress", &initial_progress);

        // Throttle progress events to max 10/sec (100ms intervals)
        let mut last_emit = Instant::now();
        let throttle_duration = Duration::from_millis(100);

        // Download with progress
        while let Some(chunk) = stream.next().await {
            // Check if download was cancelled
            if cancel_flag.load(Ordering::Relaxed) {
                // Close the file before returning
                drop(file);
                info!("Download cancelled for: {}", model_id);

                // Update state to mark as not downloading
                {
                    let mut models = self.available_models.lock().unwrap();
                    if let Some(model) = models.get_mut(model_id) {
                        model.is_downloading = false;
                    }
                }

                // Remove cancel flag
                {
                    let mut flags = self.cancel_flags.lock().unwrap();
                    flags.remove(model_id);
                }

                // Keep partial file for resume functionality
                return Ok(());
            }

            let chunk = chunk.inspect_err(|_| {
                // Mark as not downloading on error
                let mut models = self.available_models.lock().unwrap();
                if let Some(model) = models.get_mut(model_id) {
                    model.is_downloading = false;
                }
            })?;

            file.write_all(&chunk)?;
            downloaded += chunk.len() as u64;

            let percentage = if total_size > 0 {
                (downloaded as f64 / total_size as f64) * 100.0
            } else {
                0.0
            };

            // Emit progress event (throttled to avoid UI freeze)
            if last_emit.elapsed() >= throttle_duration {
                let progress = DownloadProgress {
                    model_id: model_id.to_string(),
                    downloaded,
                    total: total_size,
                    percentage,
                };
                let _ = self.app_handle.emit("model-download-progress", &progress);
                last_emit = Instant::now();
            }
        }

        // Emit final progress to ensure 100% is shown
        let final_progress = DownloadProgress {
            model_id: model_id.to_string(),
            downloaded,
            total: total_size,
            percentage: if total_size > 0 {
                (downloaded as f64 / total_size as f64) * 100.0
            } else {
                100.0
            },
        };
        let _ = self
            .app_handle
            .emit("model-download-progress", &final_progress);

        file.flush()?;
        drop(file); // Ensure file is closed before moving

        // Verify downloaded file size matches expected size
        if total_size > 0 {
            let actual_size = partial_path.metadata()?.len();
            if actual_size != total_size {
                // Download is incomplete/corrupted - delete partial and return error
                let _ = fs::remove_file(&partial_path);
                {
                    let mut models = self.available_models.lock().unwrap();
                    if let Some(model) = models.get_mut(model_id) {
                        model.is_downloading = false;
                    }
                }
                return Err(anyhow::anyhow!(
                    "Download incomplete: expected {} bytes, got {} bytes",
                    total_size,
                    actual_size
                ));
            }
        }

        // Handle directory-based models (extract tar.gz) vs file-based models
        if model_info.is_directory {
            // Track that this model is being extracted
            {
                let mut extracting = self.extracting_models.lock().unwrap();
                extracting.insert(model_id.to_string());
            }

            // Emit extraction started event
            let _ = self.app_handle.emit("model-extraction-started", model_id);
            info!("Extracting archive for directory-based model: {}", model_id);

            // Use a temporary extraction directory to ensure atomic operations
            let temp_extract_dir = self
                .models_dir
                .join(format!("{}.extracting", &model_info.filename));
            let final_model_dir = self.models_dir.join(&model_info.filename);

            // Clean up any previous incomplete extraction
            if temp_extract_dir.exists() {
                let _ = fs::remove_dir_all(&temp_extract_dir);
            }

            // Create temporary extraction directory
            fs::create_dir_all(&temp_extract_dir)?;

            // Open the downloaded tar.gz file
            let tar_gz = File::open(&partial_path)?;
            let tar = GzDecoder::new(tar_gz);
            let mut archive = Archive::new(tar);

            // Extract to the temporary directory first
            archive.unpack(&temp_extract_dir).map_err(|e| {
                let error_msg = format!("Failed to extract archive: {}", e);
                // Clean up failed extraction
                let _ = fs::remove_dir_all(&temp_extract_dir);
                // Remove from extracting set
                {
                    let mut extracting = self.extracting_models.lock().unwrap();
                    extracting.remove(model_id);
                }
                let _ = self.app_handle.emit(
                    "model-extraction-failed",
                    &serde_json::json!({
                        "model_id": model_id,
                        "error": error_msg
                    }),
                );
                anyhow::anyhow!(error_msg)
            })?;

            // Find the actual extracted directory (archive might have a nested structure)
            let extracted_dirs: Vec<_> = fs::read_dir(&temp_extract_dir)?
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
                .collect();

            if extracted_dirs.len() == 1 {
                // Single directory extracted, move it to the final location
                let source_dir = extracted_dirs[0].path();
                if final_model_dir.exists() {
                    fs::remove_dir_all(&final_model_dir)?;
                }
                fs::rename(&source_dir, &final_model_dir)?;
                // Clean up temp directory
                let _ = fs::remove_dir_all(&temp_extract_dir);
            } else {
                // Multiple items or no directories, rename the temp directory itself
                if final_model_dir.exists() {
                    fs::remove_dir_all(&final_model_dir)?;
                }
                fs::rename(&temp_extract_dir, &final_model_dir)?;
            }

            info!("Successfully extracted archive for model: {}", model_id);
            // Remove from extracting set
            {
                let mut extracting = self.extracting_models.lock().unwrap();
                extracting.remove(model_id);
            }
            // Emit extraction completed event
            let _ = self.app_handle.emit("model-extraction-completed", model_id);

            // Remove the downloaded tar.gz file
            let _ = fs::remove_file(&partial_path);
        } else {
            // Move partial file to final location for file-based models
            fs::rename(&partial_path, &model_path)?;
        }

        // Update download status
        {
            let mut models = self.available_models.lock().unwrap();
            if let Some(model) = models.get_mut(model_id) {
                model.is_downloading = false;
                model.is_downloaded = true;
                model.partial_size = 0;
            }
        }

        // Remove cancel flag on successful completion
        {
            let mut flags = self.cancel_flags.lock().unwrap();
            flags.remove(model_id);
        }

        // Emit completion event
        let _ = self.app_handle.emit("model-download-complete", model_id);

        info!(
            "Successfully downloaded model {} to {:?}",
            model_id, model_path
        );

        Ok(())
    }

    pub fn delete_model(&self, model_id: &str) -> Result<()> {
        debug!("ModelManager: delete_model called for: {}", model_id);

        let model_info = {
            let models = self.available_models.lock().unwrap();
            models.get(model_id).cloned()
        };

        let model_info =
            model_info.ok_or_else(|| anyhow::anyhow!("Model not found: {}", model_id))?;

        debug!("ModelManager: Found model info: {:?}", model_info);

        let model_path = self.models_dir.join(&model_info.filename);
        let partial_path = self
            .models_dir
            .join(format!("{}.partial", &model_info.filename));
        debug!("ModelManager: Model path: {:?}", model_path);
        debug!("ModelManager: Partial path: {:?}", partial_path);

        let mut deleted_something = false;

        if model_info.is_directory {
            // Delete complete model directory if it exists
            if model_path.exists() && model_path.is_dir() {
                info!("Deleting model directory at: {:?}", model_path);
                fs::remove_dir_all(&model_path)?;
                info!("Model directory deleted successfully");
                deleted_something = true;
            }
        } else {
            // Delete complete model file if it exists
            if model_path.exists() {
                info!("Deleting model file at: {:?}", model_path);
                fs::remove_file(&model_path)?;
                info!("Model file deleted successfully");
                deleted_something = true;
            }
        }

        // Delete partial file if it exists (same for both types)
        if partial_path.exists() {
            info!("Deleting partial file at: {:?}", partial_path);
            fs::remove_file(&partial_path)?;
            info!("Partial file deleted successfully");
            deleted_something = true;
        }

        if !deleted_something {
            return Err(anyhow::anyhow!("No model files found to delete"));
        }

        // Custom models should be removed from the list entirely since they
        // have no download URL and can't be re-downloaded
        if model_info.is_custom {
            let mut models = self.available_models.lock().unwrap();
            models.remove(model_id);
            debug!("ModelManager: removed custom model from available models");
        } else if model_id.starts_with("hf-") {
            self.available_models.lock().unwrap().remove(model_id);
            self.save_huggingface_models()?;
            debug!("ModelManager: removed Hugging Face model from available models");
        } else {
            // Update download status (marks predefined models as not downloaded)
            self.update_download_status()?;
            debug!("ModelManager: download status updated");
        }

        // Emit event to notify UI
        let _ = self.app_handle.emit("model-deleted", model_id);

        Ok(())
    }

    pub fn cancel_download(&self, model_id: &str) -> Result<()> {
        debug!("ModelManager: cancel_download called for: {}", model_id);

        // Set the cancellation flag to stop the download loop
        {
            let flags = self.cancel_flags.lock().unwrap();
            if let Some(flag) = flags.get(model_id) {
                flag.store(true, Ordering::Relaxed);
                info!("Cancellation flag set for: {}", model_id);
            } else {
                warn!("No active download found for: {}", model_id);
            }
        }

        // Update state immediately for UI responsiveness
        {
            let mut models = self.available_models.lock().unwrap();
            if let Some(model) = models.get_mut(model_id) {
                model.is_downloading = false;
            }
        }

        // Update download status to reflect current state
        self.update_download_status()?;

        // Emit cancellation event so all UI components can clear their state
        let _ = self.app_handle.emit("model-download-cancelled", model_id);

        info!("Download cancellation initiated for: {}", model_id);
        Ok(())
    }
}

pub(crate) fn kokoro_languages() -> Vec<String> {
    [
        "en", "en-gb", "es", "fr", "hi", "it", "ja", "pt", "zh-Hans", "zh-Hant", "yue",
    ]
    .iter()
    .map(|l| l.to_string())
    .collect()
}

#[derive(Deserialize)]
struct HfRepoInfo {
    siblings: Vec<HfSibling>,
}

#[derive(Deserialize)]
pub(crate) struct HfSibling {
    pub rfilename: String,
    #[serde(default)]
    pub size: Option<u64>,
}

pub(crate) fn hf_file_url(repo: &str, path: &str) -> String {
    format!("{HF_BASE_URL}/{repo}/resolve/main/{path}")
}

pub(crate) fn size_mb(bytes: u64) -> u64 {
    bytes.div_ceil(1024 * 1024)
}

pub(crate) fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

pub(crate) fn downloadable_model(
    id: String,
    name: String,
    description: String,
    engine_type: EngineType,
    supported_languages: Vec<String>,
    mut components: Vec<ModelComponent>,
    (accuracy_score, speed_score): (f32, f32),
    is_custom: bool,
) -> ModelInfo {
    components.sort_by(|a, b| a.filename.cmp(&b.filename));
    ModelInfo {
        filename: id.clone(),
        id,
        name,
        description,
        url: None,
        size_mb: components.iter().map(|c| c.size_mb).sum(),
        is_downloaded: false,
        is_downloading: false,
        partial_size: 0,
        is_directory: true,
        engine_type,
        accuracy_score,
        speed_score,
        is_recommended: false,
        supported_languages,
        is_custom,
        components,
    }
}

fn model_files_present(models_dir: &Path, model: &ModelInfo) -> bool {
    let path = models_dir.join(&model.filename);
    if model.components.is_empty() {
        path.exists()
    } else {
        model
            .components
            .iter()
            .all(|c| path.join(&c.filename).exists())
    }
}

fn load_cached_huggingface_models(models_dir: &Path) -> Vec<ModelInfo> {
    let mut models = Vec::new();
    if let Ok(json) = fs::read_to_string(models_dir.join(HF_MODELS_FILE)) {
        match serde_json::from_str::<Vec<ModelInfo>>(&json) {
            // Older builds flagged these as custom (local-only) models.
            Ok(added) => models.extend(
                added
                    .into_iter()
                    .filter(|m| m.engine_type != EngineType::Piper)
                    // Hugging Face import is not offered in the UI, so only keep
                    // models the user already has on disk.
                    .filter(|m| model_files_present(models_dir, m))
                    .map(|mut m| {
                        m.is_custom = false;
                        m
                    }),
            ),
            Err(e) => warn!("Ignoring unreadable {}: {}", HF_MODELS_FILE, e),
        }
    }
    models
}

/// Accepts `owner/name` or a huggingface.co URL and returns `owner/name`.
fn parse_hf_repo(input: &str) -> Result<String> {
    let trimmed = input.trim().trim_end_matches('/');
    let path = trimmed
        .strip_prefix("https://")
        .unwrap_or(trimmed)
        .strip_prefix("huggingface.co/")
        .unwrap_or(trimmed.strip_prefix("https://").unwrap_or(trimmed));
    let parts: Vec<&str> = path.split('/').take(2).collect();
    let valid = |part: &&str| {
        !part.is_empty()
            && !part.starts_with('.')
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    if parts.len() == 2 && parts.iter().all(valid) {
        Ok(parts.join("/"))
    } else {
        anyhow::bail!("Enter a Hugging Face model as owner/name, e.g. fastrtc/kokoro-onnx")
    }
}

/// Detect a supported model (e.g. Kokoro: `.onnx` + `voices*.bin`) among a
/// repo's files.
fn models_from_hf_repo(repo: &str, siblings: &[HfSibling]) -> Result<Vec<ModelInfo>> {
    model_families::detect(&model_families::HfRepo {
        id: repo,
        files: siblings,
    })
}

#[cfg(test)]
mod huggingface_tests {
    use super::*;

    fn sibling(path: &str, size: u64) -> HfSibling {
        HfSibling {
            rfilename: path.to_string(),
            size: Some(size),
        }
    }

    #[test]
    fn parses_repo_ids_and_urls() {
        for input in [
            "rhasspy/piper-voices",
            " https://huggingface.co/rhasspy/piper-voices/ ",
            "huggingface.co/rhasspy/piper-voices/tree/main",
        ] {
            assert_eq!(parse_hf_repo(input).unwrap(), "rhasspy/piper-voices");
        }
        for input in ["", "piper-voices", "../etc", "owner/na me", "owner/.."] {
            assert!(
                parse_hf_repo(input).is_err(),
                "{input:?} should be rejected"
            );
        }
    }

    #[test]
    fn detects_kokoro_repo_and_renames_voices_file() {
        let siblings = [
            sibling("kokoro-v1.0.onnx", 300),
            sibling("kokoro-v1.0.int8.onnx", 90),
            sibling("voices-v1.0.bin", 27),
        ];
        let models = models_from_hf_repo("fastrtc/kokoro-onnx", &siblings).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "hf-fastrtc--kokoro-onnx");
        assert!(matches!(models[0].engine_type, EngineType::Kokoro));
        let files: Vec<&str> = models[0]
            .components
            .iter()
            .map(|c| c.filename.as_str())
            .collect();
        assert_eq!(files, vec!["kokoro-v1.0.int8.onnx", "voices-v1.0.bin"]);
    }

    #[test]
    fn rejects_repos_without_supported_files() {
        let siblings = [sibling("model.safetensors", 10), sibling("config.json", 1)];
        let err = models_from_hf_repo("someone/llm", &siblings).unwrap_err();
        assert!(err.to_string().contains("no model Parrot can run"));
    }

    #[test]
    fn cached_models_keep_only_downloaded_kokoro() {
        let dir = std::env::temp_dir().join(format!("parrot-hf-cache-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut added = models_from_hf_repo(
            "me/kokoro",
            &[sibling("model.onnx", 1), sibling("voices.bin", 1)],
        )
        .unwrap();
        let mut piper = added[0].clone();
        piper.id = "hf-me--voices--a".to_string();
        piper.engine_type = EngineType::Piper;
        added.push(piper);
        let mut not_downloaded = added[0].clone();
        not_downloaded.id = "hf-me--gone".to_string();
        not_downloaded.filename = "gone".to_string();
        added.push(not_downloaded);
        // "gone" keeps an empty directory, as if its files were deleted by hand.
        for m in &added {
            let model_dir = dir.join(&m.filename);
            fs::create_dir_all(&model_dir).unwrap();
            if m.id != "hf-me--gone" {
                for c in &m.components {
                    fs::write(model_dir.join(&c.filename), b"x").unwrap();
                }
            }
        }
        fs::write(
            dir.join(HF_MODELS_FILE),
            serde_json::to_string(&added).unwrap(),
        )
        .unwrap();
        let ids: Vec<String> = load_cached_huggingface_models(&dir)
            .into_iter()
            .map(|m| m.id)
            .collect();
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(ids, vec!["hf-me--kokoro"]);
    }
}
