use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait};
use log::{debug, error, info, warn};
use rodio::buffer::SamplesBuffer;
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player};
use serde::Serialize;
use std::collections::BTreeMap;
use std::num::NonZero;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, TryLockError};
use std::thread;
use std::time::{Duration, Instant, SystemTime};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager};
use tts_rs::{
    engines::kokoro::{KokoroEngine, KokoroInferenceParams, KokoroModelParams},
    SynthesisEngine,
};

use crate::audio_feedback;
use crate::managers::history::HistoryManager;
use crate::managers::model::ModelManager;
use crate::settings::{get_settings, ModelUnloadTimeout};
use crate::text_normalization::normalize_text_for_tts;
use crate::utils::{hide_speaking_overlay, show_processing_overlay, show_speaking_overlay};

/// The model ID managed by this TTS manager.
pub const MODEL_ID: &str = "kokoro";
const MAX_PARALLEL_SYNTH_ENGINES: usize = 2;
const ENGINE_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(2);
/// Number of samples to crossfade between text-level chunks (10ms @ 24kHz).
/// Matches the crossfade length used by tts-rs for sub-chunk blending.
const CROSSFADE_SAMPLES: usize = 240;

#[derive(Clone, Debug, Serialize)]
pub struct ModelStateEvent {
    pub event_type: String,
    pub model_id: Option<String>,
    pub model_name: Option<String>,
    pub error: Option<String>,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TtsLifecycleState {
    Idle = 0,
    Processing = 1,
    Speaking = 2,
    Paused = 3,
}

struct ActiveSink {
    request_id: u64,
    sink: Arc<Player>,
}

struct ChunkSynthesisResult {
    index: usize,
    synth_elapsed_secs: f32,
    sample_rate: u32,
    samples: Vec<f32>,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug)]
struct InferenceTuning {
    target_workers: usize,
    threads_per_worker: usize,
}

pub struct TTSManager {
    engines: Arc<Vec<Arc<Mutex<Option<KokoroEngine>>>>>,
    app_handle: AppHandle,
    model_manager: Arc<ModelManager>,
    is_loading: Arc<Mutex<bool>>,
    loading_condvar: Arc<Condvar>,
    generation: Arc<AtomicU64>,
    active_request: Arc<AtomicU64>,
    lifecycle_state: Arc<AtomicU8>,
    current_sink: Arc<Mutex<Option<ActiveSink>>>,
    last_activity: Arc<AtomicU64>,
    shutdown_signal: Arc<AtomicBool>,
    espeak_ng_path: Option<PathBuf>,
    espeak_ng_data_path: Option<PathBuf>,
}

impl Drop for TTSManager {
    fn drop(&mut self) {
        self.shutdown_signal.store(true, Ordering::Relaxed);
    }
}

impl TTSManager {
    pub fn new(
        app_handle: &AppHandle,
        model_manager: Arc<ModelManager>,
        espeak_paths: (Option<PathBuf>, Option<PathBuf>),
    ) -> Result<Self> {
        let engines = Arc::new(
            (0..MAX_PARALLEL_SYNTH_ENGINES)
                .map(|_| Arc::new(Mutex::new(None)))
                .collect::<Vec<_>>(),
        );
        let is_loading = Arc::new(Mutex::new(false));
        let loading_condvar = Arc::new(Condvar::new());
        let generation = Arc::new(AtomicU64::new(0));
        let active_request = Arc::new(AtomicU64::new(0));
        let lifecycle_state = Arc::new(AtomicU8::new(TtsLifecycleState::Idle as u8));
        let current_sink = Arc::new(Mutex::new(None));
        let last_activity = Arc::new(AtomicU64::new(now_ms()));
        let shutdown_signal = Arc::new(AtomicBool::new(false));

        // Spawn idle watcher thread
        {
            let engines_clone = Arc::clone(&engines);
            let app_handle_clone = app_handle.clone();
            let last_activity_clone = Arc::clone(&last_activity);
            let shutdown_signal_clone = Arc::clone(&shutdown_signal);

            thread::spawn(move || {
                loop {
                    thread::sleep(Duration::from_secs(10));

                    if shutdown_signal_clone.load(Ordering::Relaxed) {
                        break;
                    }

                    let settings = get_settings(&app_handle_clone);
                    let timeout_seconds = settings.model_unload_timeout.to_seconds();

                    if let Some(limit_seconds) = timeout_seconds {
                        // Immediate unloading is handled directly in speak(); skip here
                        if settings.model_unload_timeout == ModelUnloadTimeout::Immediately {
                            continue;
                        }

                        let last = last_activity_clone.load(Ordering::Relaxed);
                        let now = now_ms();

                        if now.saturating_sub(last) > limit_seconds * 1000 {
                            let is_loaded = loaded_engine_count(&engines_clone) > 0;
                            if is_loaded {
                                debug!("Unloading TTS model due to inactivity");
                                for slot in engines_clone.iter() {
                                    *slot.lock().unwrap() = None;
                                }
                                let _ = app_handle_clone.emit(
                                    "model-state-changed",
                                    ModelStateEvent {
                                        event_type: "unloaded".to_string(),
                                        model_id: None,
                                        model_name: None,
                                        error: None,
                                    },
                                );
                                info!("TTS model unloaded due to inactivity");
                            }
                        }
                    }
                }
                debug!("TTS idle watcher thread shutting down gracefully");
            });
        }

        Ok(Self {
            engines,
            app_handle: app_handle.clone(),
            model_manager,
            is_loading,
            loading_condvar,
            generation,
            active_request,
            lifecycle_state,
            current_sink,
            last_activity,
            shutdown_signal,
            espeak_ng_path: espeak_paths.0,
            espeak_ng_data_path: espeak_paths.1,
        })
    }

    /// Begins a new TTS request when idle.
    /// If a request is already active, this toggles it off and returns `None`.
    pub fn begin_request_or_toggle_stop(&self) -> Option<u64> {
        if self.active_request.load(Ordering::SeqCst) != 0 {
            self.stop();
            return None;
        }

        let request_id = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.active_request.store(request_id, Ordering::SeqCst);
        self.set_lifecycle_state(TtsLifecycleState::Processing);
        Some(request_id)
    }

    pub fn is_request_active(&self, request_id: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == request_id
            && self.active_request.load(Ordering::SeqCst) == request_id
    }

    pub fn stop_if_request_active(&self, request_id: u64) -> bool {
        let is_active_request = self.active_request.load(Ordering::SeqCst) == request_id;
        if !is_active_request {
            return false;
        }

        if self
            .generation
            .compare_exchange(
                request_id,
                request_id.wrapping_add(1),
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_err()
        {
            return false;
        }

        clear_active_request_if_owned(&self.active_request, request_id);
        self.transition_to_idle_and_stop_sink();
        debug!("TTS playback stopped for request {}", request_id);
        true
    }

    /// Kick off model loading in the background. No-op if already loading or loaded.
    pub fn initiate_model_load(&self) {
        let mut is_loading = self.is_loading.lock().unwrap();
        if *is_loading || loaded_engine_count(&self.engines) > 0 {
            return;
        }
        *is_loading = true;

        let engines = Arc::clone(&self.engines);
        let is_loading_arc = Arc::clone(&self.is_loading);
        let condvar = Arc::clone(&self.loading_condvar);
        let app_handle = self.app_handle.clone();
        let model_manager = Arc::clone(&self.model_manager);
        let espeak_ng_path = self.espeak_ng_path.clone();
        let espeak_ng_data_path = self.espeak_ng_data_path.clone();

        thread::spawn(move || {
            // Resolve human-readable name from ModelManager; fall back to ID if missing.
            let model_name = model_manager
                .get_model_info(MODEL_ID)
                .map(|info| info.name)
                .unwrap_or_else(|| MODEL_ID.to_string());

            let model_dir = match resolve_kokoro_model_dir(&app_handle) {
                Ok(dir) => dir,
                Err(e) => {
                    error!("{}", e);
                    let _ = app_handle.emit("tts-error", e.clone());
                    *is_loading_arc.lock().unwrap() = false;
                    condvar.notify_all();
                    return;
                }
            };

            info!("Loading {} model from {:?}", model_name, model_dir);
            let _ = app_handle.emit(
                "model-state-changed",
                ModelStateEvent {
                    event_type: "loading_started".to_string(),
                    model_id: Some(MODEL_ID.to_string()),
                    model_name: Some(model_name.clone()),
                    error: None,
                },
            );
            // Resolve the cache path for the pre-optimized ORT graph.
            // Always stored in AppData so it works even when the source model is
            // in read-only bundled resources.  The cache is invalidated automatically
            // when the model directory is deleted (e.g. on model re-download).
            let optimized_cache_path = app_handle.path().app_data_dir().ok().map(|dir| {
                let cache_dir = dir.join("models").join("kokoro");
                let _ = std::fs::create_dir_all(&cache_dir);
                cache_dir.join("kokoro-optimized.onnx")
            });
            let tts_settings = get_settings(&app_handle);
            let tuning = if tts_settings.tts_workers > 0 {
                let manual_workers = tts_settings.tts_workers.min(MAX_PARALLEL_SYNTH_ENGINES);
                let cpu_count = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(2);
                let threads_per_worker = (cpu_count / manual_workers).max(1);
                InferenceTuning {
                    target_workers: manual_workers,
                    threads_per_worker,
                }
            } else {
                infer_kokoro_tuning()
            };
            info!(
                "Kokoro tuning: target_workers={}, threads_per_worker={} (manual={})",
                tuning.target_workers,
                tuning.threads_per_worker,
                tts_settings.tts_workers > 0
            );

            let mut loaded_workers = 0usize;
            let mut first_error: Option<String> = None;
            let target_workers = tuning.target_workers.min(engines.len());

            for (worker_index, slot) in engines.iter().enumerate() {
                if worker_index >= target_workers {
                    *slot.lock().unwrap() = None;
                    continue;
                }

                let mut kokoro =
                    KokoroEngine::with_espeak(espeak_ng_path.clone(), espeak_ng_data_path.clone());
                match kokoro.load_model_with_params(
                    &model_dir,
                    KokoroModelParams {
                        num_threads: Some(tuning.threads_per_worker),
                        optimized_model_cache_path: optimized_cache_path.clone(),
                    },
                ) {
                    Ok(()) => {
                        info!(
                            "Warming up {} pipeline worker {}/{} (espeak-ng + ORT)...",
                            model_name,
                            worker_index + 1,
                            target_workers
                        );
                        let _ = kokoro.synthesize("Hello.", None);
                        *slot.lock().unwrap() = Some(kokoro);
                        loaded_workers += 1;
                    }
                    Err(e) => {
                        let worker_error = format!(
                            "Failed to load {} worker {}/{} from {}: {}",
                            model_name,
                            worker_index + 1,
                            target_workers,
                            model_dir.display(),
                            e
                        );
                        error!("{}", worker_error);
                        first_error.get_or_insert(worker_error);
                        *slot.lock().unwrap() = None;
                    }
                }
            }

            if loaded_workers == 0 {
                hide_speaking_overlay(&app_handle);
                let error_msg = first_error.unwrap_or_else(|| {
                    format!(
                        "Failed to load {} model from {}",
                        model_name,
                        model_dir.display()
                    )
                });
                let _ = app_handle.emit(
                    "model-state-changed",
                    ModelStateEvent {
                        event_type: "loading_failed".to_string(),
                        model_id: Some(MODEL_ID.to_string()),
                        model_name: Some(model_name),
                        error: Some(error_msg.clone()),
                    },
                );
                let _ = app_handle.emit("tts-error", error_msg);
            } else {
                info!(
                    "{} model loaded successfully with {}/{} synthesis workers",
                    model_name, loaded_workers, target_workers
                );
                let _ = app_handle.emit(
                    "model-state-changed",
                    ModelStateEvent {
                        event_type: "loaded".to_string(),
                        model_id: Some(MODEL_ID.to_string()),
                        model_name: Some(model_name),
                        error: None,
                    },
                );
            }
            *is_loading_arc.lock().unwrap() = false;
            condvar.notify_all();
        });
    }

    /// Stop any in-progress TTS playback or processing.
    pub fn stop(&self) {
        let request_id = self.active_request.swap(0, Ordering::SeqCst);
        if request_id != 0 {
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
        self.transition_to_idle_and_stop_sink();
        debug!("TTS playback stopped");
    }

    pub fn is_model_loaded(&self) -> bool {
        loaded_engine_count(&self.engines) > 0
    }

    pub fn is_model_loading(&self) -> bool {
        *self.is_loading.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Returns all available Kokoro voices in sorted order.
    /// Triggers loading when needed and waits for the load cycle to finish.
    pub fn get_available_voices(&self) -> Result<Vec<String>> {
        if !self.is_model_loaded() {
            self.initiate_model_load();
            self.wait_for_pending_model_load()?;
        }

        let synthesis_engines = loaded_engine_slots(&self.engines);
        if synthesis_engines.is_empty() {
            return Err(anyhow!(
                "Kokoro model is not loaded. Install model files and try again."
            ));
        }

        let voices = collect_available_voices(&synthesis_engines);

        if voices.is_empty() {
            return Err(anyhow!("No Kokoro voices were found in the loaded model."));
        }

        Ok(voices)
    }

    /// Returns `Some(MODEL_ID)` when the model is loaded, `None` otherwise.
    pub fn get_current_model(&self) -> Option<String> {
        if self.is_model_loaded() {
            Some(MODEL_ID.to_string())
        } else {
            None
        }
    }

    /// Unload the Kokoro engine from memory and emit a model-state-changed event.
    pub fn unload_model(&self) -> Result<()> {
        debug!("Unloading TTS model");
        for slot in self.engines.iter() {
            *slot.lock().unwrap() = None;
        }
        let _ = self.app_handle.emit(
            "model-state-changed",
            ModelStateEvent {
                event_type: "unloaded".to_string(),
                model_id: None,
                model_name: None,
                error: None,
            },
        );
        info!("TTS model unloaded");
        Ok(())
    }

    /// Unloads the model immediately if the ModelUnloadTimeout setting is set to Immediately.
    pub fn maybe_unload_immediately(&self, context: &str) {
        let settings = get_settings(&self.app_handle);
        if settings.model_unload_timeout == ModelUnloadTimeout::Immediately
            && self.is_model_loaded()
        {
            info!("Immediately unloading TTS model after {}", context);
            if let Err(e) = self.unload_model() {
                warn!("Failed to immediately unload TTS model: {}", e);
            }
        }
    }

    /// Pause/resume active speech.
    /// Returns `true` when playback is paused, `false` when resumed.
    pub fn toggle_pause(&self) -> Result<bool> {
        let sink_guard = self.current_sink.lock().unwrap();
        let Some(active_sink) = sink_guard.as_ref() else {
            return Err(anyhow!("No active TTS playback"));
        };
        let player = &active_sink.sink;

        match self.current_lifecycle_state() {
            TtsLifecycleState::Speaking => {
                player.pause();
                self.set_lifecycle_state(TtsLifecycleState::Paused);
                Ok(true)
            }
            TtsLifecycleState::Paused => {
                player.play();
                self.set_lifecycle_state(TtsLifecycleState::Speaking);
                Ok(false)
            }
            _ => Err(anyhow!("TTS playback is not in a pausable state")),
        }
    }

    /// Synthesize and play for a previously-started request.
    pub fn speak(&self, text: String, request_id: u64) {
        // Record activity timestamp for idle watcher
        self.last_activity.store(now_ms(), Ordering::Relaxed);

        let engines = Arc::clone(&self.engines);
        let is_loading = Arc::clone(&self.is_loading);
        let condvar = Arc::clone(&self.loading_condvar);
        let generation = Arc::clone(&self.generation);
        let active_request = Arc::clone(&self.active_request);
        let lifecycle_state = Arc::clone(&self.lifecycle_state);
        let current_sink = Arc::clone(&self.current_sink);
        let app_handle = self.app_handle.clone();

        thread::spawn(move || {
            let started_at = Instant::now();

            {
                let mut loading = is_loading.lock().unwrap();
                while *loading {
                    loading = condvar.wait(loading).unwrap();
                }
            }

            if generation.load(Ordering::SeqCst) != request_id
                || active_request.load(Ordering::SeqCst) != request_id
            {
                return;
            }

            if loaded_engine_count(&engines) == 0 {
                let message = "Kokoro model is not loaded. Install model files and try again.";
                error!("{}", message);
                if set_idle_and_cleanup_for_request(
                    &generation,
                    &active_request,
                    request_id,
                    &lifecycle_state,
                    &current_sink,
                    &app_handle,
                ) {
                    let _ = app_handle.emit("tts-error", message);
                }
                return;
            }

            // Show "Processing..." overlay immediately so the user gets feedback
            // before the first audio chunk is synthesized.
            show_processing_overlay(&app_handle);

            let tts_settings = get_settings(&app_handle);
            let tts_speed = tts_settings.tts_speed;
            let shorten_first_chunk = tts_settings.tts_shorten_first_chunk;
            let normalized_text = normalize_text_for_tts(&text);

            debug!(
                "Prepared {} input chars into {} chars for TTS",
                text.len(),
                normalized_text.len()
            );

            let chunks = split_text_for_playback(&normalized_text, shorten_first_chunk);
            if chunks.is_empty() {
                let message = "No readable text found for TTS.";
                error!("{}", message);
                if set_idle_and_cleanup_for_request(
                    &generation,
                    &active_request,
                    request_id,
                    &lifecycle_state,
                    &current_sink,
                    &app_handle,
                ) {
                    let _ = app_handle.emit("tts-error", message);
                }
                return;
            }

            let _stream = match open_output_stream(&app_handle) {
                Ok(s) => s,
                Err(e) => {
                    error!("{}", e);
                    if set_idle_and_cleanup_for_request(
                        &generation,
                        &active_request,
                        request_id,
                        &lifecycle_state,
                        &current_sink,
                        &app_handle,
                    ) {
                        let _ = app_handle.emit("tts-error", e);
                    }
                    return;
                }
            };

            let player = Player::connect_new(_stream.mixer());
            player.pause();
            let shared_sink = Arc::new(player);

            {
                let mut sink_guard = current_sink.lock().unwrap();
                if generation.load(Ordering::SeqCst) != request_id
                    || active_request.load(Ordering::SeqCst) != request_id
                {
                    return;
                }
                *sink_guard = Some(ActiveSink {
                    request_id,
                    sink: shared_sink.clone(),
                });
            }

            let synthesis_engines = loaded_engine_slots(&engines);
            if synthesis_engines.is_empty() {
                let message = "Kokoro model is not loaded. Install model files and try again.";
                error!("{}", message);
                if set_idle_and_cleanup_for_request(
                    &generation,
                    &active_request,
                    request_id,
                    &lifecycle_state,
                    &current_sink,
                    &app_handle,
                ) {
                    let _ = app_handle.emit("tts-error", message);
                }
                return;
            }

            let chunks = Arc::new(chunks);
            let total_chunks = chunks.len();
            let total_chars: usize = chunks.iter().map(|c| c.len()).sum();
            let shared_style_index = estimate_kokoro_style_index(chunks.as_ref());
            let resolved_language: Option<String> = if tts_settings.selected_language == "auto" {
                tauri_plugin_os::locale().map(|l| l.replace('_', "-"))
            } else {
                Some(tts_settings.selected_language.clone())
            };
            let selected_voice_override = tts_settings.selected_kokoro_voice.as_deref();
            let max_active_workers = total_chunks.max(1).min(synthesis_engines.len());
            let synthesis_engines: Vec<_> = synthesis_engines
                .into_iter()
                .take(max_active_workers)
                .collect();
            let available_voices = collect_available_voices(&synthesis_engines);
            let selected_voice = select_kokoro_voice_for_text(
                resolved_language.as_deref(),
                selected_voice_override,
                &available_voices,
            );
            let worker_count = synthesis_engines.len();
            let mut total_synth_secs = 0.0_f32;
            let mut started_playback = false;
            let mut buffered_seconds = 0.0_f32;
            let mut total_audio_seconds = 0.0_f32;
            debug!(
                "TTS split into {} chunks ({} chars) using {} synthesis workers (style_index={}, voice={})",
                total_chunks, total_chars, worker_count, shared_style_index, selected_voice
            );

            let mut collected_samples: Vec<f32> = Vec::new();
            let mut collected_sample_rate: u32 = 24000;

            let next_chunk_to_synthesize = Arc::new(AtomicUsize::new(0));
            let channel_capacity = worker_count.saturating_mul(2).max(1);
            let (result_tx, result_rx) =
                mpsc::sync_channel::<ChunkSynthesisResult>(channel_capacity);

            for synthesis_engine in synthesis_engines {
                let chunks_for_worker = Arc::clone(&chunks);
                let next_chunk_for_worker = Arc::clone(&next_chunk_to_synthesize);
                let generation_for_worker = Arc::clone(&generation);
                let active_request_for_worker = Arc::clone(&active_request);
                let tx_for_worker = result_tx.clone();
                let style_index_for_worker = shared_style_index;
                let voice_for_worker = selected_voice.clone();
                let speed_for_worker = tts_speed;

                thread::spawn(move || loop {
                    if !request_is_active(
                        &generation_for_worker,
                        &active_request_for_worker,
                        request_id,
                    ) {
                        break;
                    }

                    let chunk_index = next_chunk_for_worker.fetch_add(1, Ordering::SeqCst);
                    if chunk_index >= chunks_for_worker.len() {
                        break;
                    }

                    let chunk = &chunks_for_worker[chunk_index];
                    let synth_start = Instant::now();

                    // Take the engine out of its slot so the mutex is released
                    // during the (potentially long) synthesize() call. This lets
                    // new requests acquire the engine immediately after cancel.
                    let mut engine = match take_engine_for_active_request(
                        &synthesis_engine,
                        &generation_for_worker,
                        &active_request_for_worker,
                        request_id,
                    ) {
                        Some(e) => e,
                        None => break,
                    };

                    if !request_is_active(
                        &generation_for_worker,
                        &active_request_for_worker,
                        request_id,
                    ) {
                        return_engine_to_slot(&synthesis_engine, engine);
                        break;
                    }

                    // Synthesize WITHOUT holding the mutex lock.
                    let synth_result = engine
                        .synthesize(
                            chunk,
                            Some(KokoroInferenceParams {
                                voice: voice_for_worker.clone(),
                                style_index: Some(style_index_for_worker),
                                speed: speed_for_worker,
                            }),
                        )
                        .map_err(|e| format!("TTS synthesis failed: {}", e));

                    // Put engine back (brief lock, just a swap).
                    return_engine_to_slot(&synthesis_engine, engine);

                    let synth_elapsed_secs = synth_start.elapsed().as_secs_f32();
                    let result = match synth_result {
                        Ok(synthesis) => ChunkSynthesisResult {
                            index: chunk_index,
                            synth_elapsed_secs,
                            sample_rate: synthesis.sample_rate,
                            samples: synthesis.samples,
                            error: None,
                        },
                        Err(err) => ChunkSynthesisResult {
                            index: chunk_index,
                            synth_elapsed_secs,
                            sample_rate: 0,
                            samples: vec![],
                            error: Some(err),
                        },
                    };

                    if !request_is_active(
                        &generation_for_worker,
                        &active_request_for_worker,
                        request_id,
                    ) {
                        break;
                    }

                    if tx_for_worker.send(result).is_err() {
                        break;
                    }
                });
            }
            drop(result_tx);

            let mut pending_results: BTreeMap<usize, ChunkSynthesisResult> = BTreeMap::new();
            let mut next_chunk_to_append = 0usize;
            let mut crossfade_tail: Option<Vec<f32>> = None;
            // Channel for sending (chunk_index, duration_secs) to the overlay
            // text updater thread as chunks are appended to the audio sink.
            let (chunk_dur_tx, chunk_dur_rx) = mpsc::channel::<(usize, f32)>();
            let mut chunk_dur_rx = Some(chunk_dur_rx);

            while next_chunk_to_append < total_chunks {
                if generation.load(Ordering::SeqCst) != request_id
                    || active_request.load(Ordering::SeqCst) != request_id
                {
                    shared_sink.stop();
                    clear_active_request_if_owned(&active_request, request_id);
                    clear_sink_if_owned_by_request(&current_sink, request_id);
                    return;
                }

                let chunk_result = match result_rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(result) => result,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        let message = "TTS synthesis workers disconnected unexpectedly.";
                        error!("{}", message);
                        if set_idle_and_cleanup_for_request(
                            &generation,
                            &active_request,
                            request_id,
                            &lifecycle_state,
                            &current_sink,
                            &app_handle,
                        ) {
                            let _ = app_handle.emit("tts-error", message);
                        }
                        return;
                    }
                };

                pending_results.insert(chunk_result.index, chunk_result);

                while let Some(chunk_result) = pending_results.remove(&next_chunk_to_append) {
                    if let Some(err) = chunk_result.error {
                        error!("{}", err);
                        if set_idle_and_cleanup_for_request(
                            &generation,
                            &active_request,
                            request_id,
                            &lifecycle_state,
                            &current_sink,
                            &app_handle,
                        ) {
                            let _ = app_handle.emit("tts-error", err);
                        }
                        return;
                    }

                    total_synth_secs += chunk_result.synth_elapsed_secs;

                    if generation.load(Ordering::SeqCst) != request_id
                        || active_request.load(Ordering::SeqCst) != request_id
                    {
                        shared_sink.stop();
                        clear_active_request_if_owned(&active_request, request_id);
                        clear_sink_if_owned_by_request(&current_sink, request_id);
                        return;
                    }

                    if chunk_result.samples.is_empty() {
                        next_chunk_to_append += 1;
                        continue;
                    }

                    let chunk_audio_seconds =
                        chunk_result.samples.len() as f32 / chunk_result.sample_rate as f32;
                    total_audio_seconds += chunk_audio_seconds;
                    buffered_seconds += chunk_audio_seconds;
                    collected_sample_rate = chunk_result.sample_rate;
                    collected_samples.extend_from_slice(&chunk_result.samples);

                    // Crossfade with the previous chunk's tail to eliminate clicks
                    // at chunk boundaries (10ms @ 24kHz = 240 samples).
                    let mut samples = chunk_result.samples;
                    if let Some(prev_tail) = crossfade_tail.take() {
                        apply_crossfade(&prev_tail, &mut samples);
                    }
                    // Hold back the last CROSSFADE_SAMPLES for blending with the next chunk
                    if samples.len() > CROSSFADE_SAMPLES {
                        let split = samples.len() - CROSSFADE_SAMPLES;
                        crossfade_tail = Some(samples[split..].to_vec());
                        samples.truncate(split);
                    }
                    shared_sink.append(SamplesBuffer::new(
                        NonZero::new(1u16).unwrap(),
                        NonZero::new(chunk_result.sample_rate).unwrap(),
                        samples,
                    ));
                    // Feed the overlay text updater thread so it can schedule
                    // when to show each chunk's text during playback.
                    let _ = chunk_dur_tx.send((next_chunk_to_append, chunk_audio_seconds));

                    if !started_playback {
                        if generation.load(Ordering::SeqCst) != request_id
                            || active_request.load(Ordering::SeqCst) != request_id
                        {
                            shared_sink.stop();
                            clear_active_request_if_owned(&active_request, request_id);
                            clear_sink_if_owned_by_request(&current_sink, request_id);
                            return;
                        }
                        started_playback = true;
                        lifecycle_state.store(TtsLifecycleState::Speaking as u8, Ordering::SeqCst);
                        show_speaking_overlay(&app_handle, chunks.first().cloned());
                        crate::shortcut::register_play_pause_shortcut(&app_handle);
                        audio_feedback::play_sound(&app_handle, audio_feedback::SoundType::Start);
                        shared_sink.play();

                        // Spawn overlay text updater: sleeps through each chunk's
                        // audio duration and emits the next chunk's text at the
                        // right moment, pausing the timer when TTS is paused.
                        if let Some(rx) = chunk_dur_rx.take() {
                            let overlay_chunks = Arc::clone(&chunks);
                            let overlay_lifecycle = Arc::clone(&lifecycle_state);
                            let overlay_generation = Arc::clone(&generation);
                            let overlay_app = app_handle.clone();
                            thread::spawn(move || {
                                overlay_text_updater(
                                    rx,
                                    &overlay_chunks,
                                    &overlay_lifecycle,
                                    &overlay_generation,
                                    request_id,
                                    &overlay_app,
                                );
                            });
                        }

                        let overall_rtf = total_synth_secs / buffered_seconds.max(0.001);
                        info!(
                            "TTS playback started in {}ms (buffered={:.2}s, rtf={:.2}, chunks={}/{}, workers={})",
                            started_at.elapsed().as_millis(),
                            buffered_seconds,
                            overall_rtf,
                            next_chunk_to_append + 1,
                            total_chunks,
                            worker_count,
                        );
                    }

                    debug!(
                        "TTS chunk {}/{} synthesized in {}ms ({:.2}s audio, rtf={:.2})",
                        next_chunk_to_append + 1,
                        total_chunks,
                        (chunk_result.synth_elapsed_secs * 1000.0) as u64,
                        chunk_audio_seconds,
                        chunk_result.synth_elapsed_secs / chunk_audio_seconds.max(0.001),
                    );

                    next_chunk_to_append += 1;
                }
            }

            // Drop the sender so the overlay updater thread knows no more chunks are coming.
            drop(chunk_dur_tx);

            // Flush any held-back crossfade tail from the final chunk
            if let Some(tail) = crossfade_tail.take() {
                shared_sink.append(SamplesBuffer::new(
                    NonZero::new(1u16).unwrap(),
                    NonZero::new(collected_sample_rate).unwrap(),
                    tail,
                ));
            }

            if !started_playback {
                let message = "TTS generated empty audio output.";
                error!("{}", message);
                if set_idle_and_cleanup_for_request(
                    &generation,
                    &active_request,
                    request_id,
                    &lifecycle_state,
                    &current_sink,
                    &app_handle,
                ) {
                    let _ = app_handle.emit("tts-error", message);
                }
                return;
            }

            while !shared_sink.empty() {
                if generation.load(Ordering::SeqCst) != request_id
                    || active_request.load(Ordering::SeqCst) != request_id
                {
                    shared_sink.stop();
                    clear_active_request_if_owned(&active_request, request_id);
                    clear_sink_if_owned_by_request(&current_sink, request_id);
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }

            if generation.load(Ordering::SeqCst) == request_id
                && active_request.load(Ordering::SeqCst) == request_id
            {
                clear_active_request_if_owned(&active_request, request_id);
                lifecycle_state.store(TtsLifecycleState::Idle as u8, Ordering::SeqCst);
                clear_sink_if_owned_by_request(&current_sink, request_id);
                hide_speaking_overlay(&app_handle);
                crate::shortcut::unregister_play_pause_shortcut(&app_handle);
                audio_feedback::play_sound(&app_handle, audio_feedback::SoundType::Stop);
                info!(
                    "TTS playback finished in {}ms ({:.2}s audio)",
                    started_at.elapsed().as_millis(),
                    total_audio_seconds
                );

                // Save to history
                if !collected_samples.is_empty() {
                    let history_manager = app_handle.state::<Arc<HistoryManager>>();
                    if let Err(e) = history_manager.save_transcription(
                        collected_samples,
                        text.clone(),
                        collected_sample_rate,
                    ) {
                        error!("Failed to save TTS history entry: {}", e);
                    }
                }
            } else {
                clear_active_request_if_owned(&active_request, request_id);
                clear_sink_if_owned_by_request(&current_sink, request_id);
            }
        });
    }

    fn current_lifecycle_state(&self) -> TtsLifecycleState {
        match self.lifecycle_state.load(Ordering::SeqCst) {
            1 => TtsLifecycleState::Processing,
            2 => TtsLifecycleState::Speaking,
            3 => TtsLifecycleState::Paused,
            _ => TtsLifecycleState::Idle,
        }
    }

    fn set_lifecycle_state(&self, state: TtsLifecycleState) {
        self.lifecycle_state.store(state as u8, Ordering::SeqCst);
    }

    fn transition_to_idle_and_stop_sink(&self) {
        self.active_request.store(0, Ordering::SeqCst);
        self.set_lifecycle_state(TtsLifecycleState::Idle);

        if let Ok(mut sink_guard) = self.current_sink.lock() {
            if let Some(active_sink) = sink_guard.take() {
                active_sink.sink.stop();
            }
        }

        hide_speaking_overlay(&self.app_handle);
    }

    fn wait_for_pending_model_load(&self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut loading = self.is_loading.lock().unwrap_or_else(|p| p.into_inner());
        while *loading {
            let now = Instant::now();
            if now >= deadline {
                return Err(anyhow!(
                    "Timed out while waiting for Kokoro model loading to finish."
                ));
            }

            let remaining = deadline.saturating_duration_since(now);
            let (next_loading, wait_result) = self
                .loading_condvar
                .wait_timeout(loading, remaining)
                .unwrap_or_else(|p| p.into_inner());
            loading = next_loading;

            if wait_result.timed_out() && *loading {
                return Err(anyhow!(
                    "Timed out while waiting for Kokoro model loading to finish."
                ));
            }
        }

        Ok(())
    }
}

/// Runs on a dedicated thread. Receives `(chunk_index, duration_secs)` from the
/// synthesis loop and emits `overlay-text` events timed to when each chunk
/// actually starts playing, so the overlay shows the text being read aloud.
///
/// The timer pauses when TTS playback is paused and exits when the request
/// is cancelled or all chunks have been emitted.
fn overlay_text_updater(
    rx: mpsc::Receiver<(usize, f32)>,
    chunks: &[String],
    lifecycle_state: &AtomicU8,
    generation: &AtomicU64,
    request_id: u64,
    app_handle: &AppHandle,
) {
    // Collect chunk durations as they arrive from the synthesis loop.
    // We emit each chunk's text *before* sleeping through its duration,
    // which correctly handles both the normal case (chunks pre-buffered)
    // and the "gap" case (synthesis lagging behind playback).
    let mut pending: Vec<(usize, f32)> = Vec::new();
    let mut current: usize = 0; // index of the chunk we're about to sleep through

    // Chunk 0's text is already shown by show_speaking_overlay.

    loop {
        if generation.load(Ordering::SeqCst) != request_id {
            return;
        }

        // Drain any newly arrived chunk durations
        while let Ok(entry) = rx.try_recv() {
            pending.push(entry);
        }

        // Wait until we have timing info for the current chunk
        if current >= pending.len() {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(entry) => pending.push(entry),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
            if current >= pending.len() {
                continue;
            }
        }

        // Emit the current chunk's text. Chunk 0 is already displayed by
        // show_speaking_overlay; for subsequent chunks this fires right
        // when the chunk starts playing — whether it was pre-buffered or
        // arrived after a synthesis gap.
        if current > 0 {
            let (chunk_idx, _) = pending[current];
            if let Some(text) = chunks.get(chunk_idx) {
                let _ = app_handle.emit("overlay-text", text.as_str());
            }
        }

        // Sleep through the current chunk's audio duration
        let (_chunk_idx, duration_secs) = pending[current];
        let sleep_target = Duration::from_secs_f32(duration_secs);
        let mut slept = Duration::ZERO;

        while slept < sleep_target {
            if generation.load(Ordering::SeqCst) != request_id {
                return;
            }

            let state = lifecycle_state.load(Ordering::SeqCst);
            if state == TtsLifecycleState::Paused as u8 {
                thread::sleep(Duration::from_millis(50));
                continue;
            }
            if state != TtsLifecycleState::Speaking as u8 {
                return;
            }

            let step = Duration::from_millis(50).min(sleep_target - slept);
            thread::sleep(step);
            slept += step;
        }

        current += 1;
    }
}

/// Linear crossfade: blend `prev_tail` into the beginning of `samples`.
/// If `samples` is shorter than `prev_tail`, only the overlapping portion is blended
/// and the non-overlapping prefix of `prev_tail` is prepended.
fn apply_crossfade(prev_tail: &[f32], samples: &mut Vec<f32>) {
    let overlap = prev_tail.len().min(samples.len());
    for i in 0..overlap {
        let t = (i + 1) as f32 / (overlap + 1) as f32;
        samples[i] = prev_tail[prev_tail.len() - overlap + i] * (1.0 - t) + samples[i] * t;
    }
    if prev_tail.len() > overlap {
        let prefix = &prev_tail[..prev_tail.len() - overlap];
        samples.splice(0..0, prefix.iter().copied());
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn infer_kokoro_tuning() -> InferenceTuning {
    let cpu_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    infer_kokoro_tuning_for_cpu_count(cpu_count)
}

fn infer_kokoro_tuning_for_cpu_count(cpu_count: usize) -> InferenceTuning {
    // Keep one core free on larger systems so UI/audio and background tasks remain smooth.
    let reserved_cores = if cpu_count >= 4 { 1 } else { 0 };
    let compute_budget = cpu_count.saturating_sub(reserved_cores).max(1);

    // On constrained hardware, use one worker to avoid contention. On stronger systems,
    // scale up to MAX_PARALLEL_SYNTH_ENGINES workers for low-latency overlap.
    let target_workers = if compute_budget < 3 {
        1
    } else {
        MAX_PARALLEL_SYNTH_ENGINES.min(compute_budget)
    };
    let threads_per_worker = (compute_budget / target_workers).max(1);

    InferenceTuning {
        target_workers,
        threads_per_worker,
    }
}

fn loaded_engine_count(engines: &Arc<Vec<Arc<Mutex<Option<KokoroEngine>>>>>) -> usize {
    engines
        .iter()
        .filter(|slot| slot.lock().map(|guard| guard.is_some()).unwrap_or(false))
        .count()
}

fn loaded_engine_slots(
    engines: &Arc<Vec<Arc<Mutex<Option<KokoroEngine>>>>>,
) -> Vec<Arc<Mutex<Option<KokoroEngine>>>> {
    let mut loaded = Vec::new();
    for slot in engines.iter() {
        if slot.lock().map(|guard| guard.is_some()).unwrap_or(false) {
            loaded.push(Arc::clone(slot));
        }
    }
    loaded
}

fn collect_available_voices(synthesis_engines: &[Arc<Mutex<Option<KokoroEngine>>>]) -> Vec<String> {
    let mut voices = Vec::new();

    for engine_slot in synthesis_engines {
        let guard = engine_slot.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(engine) = guard.as_ref() {
            voices.extend(
                engine
                    .list_voices()
                    .into_iter()
                    .map(|voice| voice.to_string()),
            );
        }
    }

    voices.sort_unstable();
    voices.dedup();
    voices.retain(|voice| !voice.trim().is_empty());
    voices
}

fn request_is_active(
    generation: &Arc<AtomicU64>,
    active_request: &Arc<AtomicU64>,
    request_id: u64,
) -> bool {
    generation.load(Ordering::SeqCst) == request_id
        && active_request.load(Ordering::SeqCst) == request_id
}

/// Take the engine out of its slot so synthesis can proceed without holding the mutex.
/// Returns `None` if the request was cancelled while waiting.
/// The caller is responsible for putting the engine back after synthesis.
fn take_engine_for_active_request(
    engine_slot: &Arc<Mutex<Option<KokoroEngine>>>,
    generation: &Arc<AtomicU64>,
    active_request: &Arc<AtomicU64>,
    request_id: u64,
) -> Option<KokoroEngine> {
    loop {
        if !request_is_active(generation, active_request, request_id) {
            return None;
        }

        match engine_slot.try_lock() {
            Ok(mut guard) => {
                if let Some(engine) = guard.take() {
                    return Some(engine);
                }
                // Engine was taken by another worker; wait for it to be put back.
                drop(guard);
                thread::sleep(ENGINE_LOCK_POLL_INTERVAL);
            }
            Err(TryLockError::WouldBlock) => thread::sleep(ENGINE_LOCK_POLL_INTERVAL),
            Err(TryLockError::Poisoned(poisoned)) => return poisoned.into_inner().take(),
        }
    }
}

/// Put the engine back into its slot after synthesis.
fn return_engine_to_slot(engine_slot: &Arc<Mutex<Option<KokoroEngine>>>, engine: KokoroEngine) {
    match engine_slot.lock() {
        Ok(mut guard) => *guard = Some(engine),
        Err(poisoned) => *poisoned.into_inner() = Some(engine),
    }
}

fn set_idle_and_cleanup_for_request(
    generation: &Arc<AtomicU64>,
    active_request: &Arc<AtomicU64>,
    request_id: u64,
    lifecycle_state: &Arc<AtomicU8>,
    current_sink: &Arc<Mutex<Option<ActiveSink>>>,
    app_handle: &AppHandle,
) -> bool {
    if generation.load(Ordering::SeqCst) != request_id
        || active_request.load(Ordering::SeqCst) != request_id
    {
        clear_active_request_if_owned(active_request, request_id);
        clear_sink_if_owned_by_request(current_sink, request_id);
        return false;
    }

    clear_active_request_if_owned(active_request, request_id);
    lifecycle_state.store(TtsLifecycleState::Idle as u8, Ordering::SeqCst);
    clear_sink_if_owned_by_request(current_sink, request_id);
    hide_speaking_overlay(app_handle);
    true
}

fn clear_active_request_if_owned(active_request: &Arc<AtomicU64>, request_id: u64) {
    let _ = active_request.compare_exchange(request_id, 0, Ordering::SeqCst, Ordering::SeqCst);
}

fn estimate_kokoro_style_index(chunks: &[String]) -> usize {
    chunks
        .iter()
        .map(|chunk| chunk.chars().filter(|ch| !ch.is_whitespace()).count())
        .sum::<usize>()
        .max(1)
}

fn select_kokoro_voice_for_text(
    language_hint: Option<&str>,
    selected_voice_override: Option<&str>,
    available_voices: &[String],
) -> String {
    if let Some(preferred_voice) =
        resolve_preferred_voice_selection(selected_voice_override, available_voices)
    {
        return preferred_voice;
    }

    let preferred_language = normalize_kokoro_language_code(language_hint).unwrap_or("en-us");

    select_voice_for_language(preferred_language, available_voices)
}

fn resolve_preferred_voice_selection(
    preferred_voice: Option<&str>,
    available_voices: &[String],
) -> Option<String> {
    let preferred = preferred_voice?.trim();
    if preferred.is_empty() {
        return None;
    }

    available_voices
        .iter()
        .find(|voice| voice.eq_ignore_ascii_case(preferred))
        .cloned()
}

fn select_voice_for_language(language: &str, available_voices: &[String]) -> String {
    let mut canonical_voices: Vec<&str> = available_voices
        .iter()
        .filter_map(|voice| {
            let trimmed = voice.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .collect();
    canonical_voices.sort_unstable();
    canonical_voices.dedup();

    let prefixes = voice_prefixes_for_language(language);
    let fallback_prefixes = voice_prefixes_for_language("en-us");

    for prefix in prefixes {
        if let Some(voice) = canonical_voices
            .iter()
            .find(|voice| voice.starts_with(prefix))
            .copied()
        {
            return voice.to_string();
        }
    }

    for prefix in fallback_prefixes {
        if let Some(voice) = canonical_voices
            .iter()
            .find(|voice| voice.starts_with(prefix))
            .copied()
        {
            return voice.to_string();
        }
    }

    canonical_voices
        .first()
        .map(|voice| voice.to_string())
        .unwrap_or_else(|| "af_heart".to_string())
}

fn voice_prefixes_for_language(language: &str) -> &'static [&'static str] {
    match language {
        "en-gb" => &["bf_", "bm_"],
        "es" => &["ef_", "em_"],
        "fr" => &["ff_"],
        "hi" => &["hf_", "hm_"],
        "it" => &["if_", "im_"],
        "ja" => &["jf_", "jm_"],
        "pt-br" => &["pf_", "pm_"],
        "cmn" => &["zf_", "zm_"],
        _ => &["af_", "am_"],
    }
}

fn normalize_kokoro_language_code(language: Option<&str>) -> Option<&'static str> {
    let raw = language?.trim();
    if raw.is_empty() {
        return None;
    }

    let normalized = raw.to_ascii_lowercase().replace('_', "-");
    let base = normalized.split('-').next().unwrap_or_default();

    if matches!(normalized.as_str(), "en-gb" | "en-uk") {
        return Some("en-gb");
    }
    if matches!(
        normalized.as_str(),
        "zh-hans"
            | "zh-hant"
            | "zh-cn"
            | "zh-tw"
            | "zh-hk"
            | "cmn"
            | "cmn-hans"
            | "cmn-hant"
            | "yue"
            | "yue-hk"
    ) {
        return Some("cmn");
    }
    if normalized == "pt-br" {
        return Some("pt-br");
    }

    match base {
        "en" => Some("en-us"),
        "es" => Some("es"),
        "fr" => Some("fr"),
        "hi" => Some("hi"),
        "it" => Some("it"),
        "ja" | "jp" => Some("ja"),
        "pt" => Some("pt-br"),
        "zh" => Some("cmn"),
        _ => None,
    }
}

fn clear_sink_if_owned_by_request(current_sink: &Arc<Mutex<Option<ActiveSink>>>, request_id: u64) {
    if let Ok(mut sink_guard) = current_sink.lock() {
        let is_owned = sink_guard
            .as_ref()
            .map(|active_sink| active_sink.request_id == request_id)
            .unwrap_or(false);
        if !is_owned {
            return;
        }

        if let Some(active_sink) = sink_guard.take() {
            active_sink.sink.stop();
        }
    }
}

fn resolve_kokoro_model_dir(app_handle: &AppHandle) -> std::result::Result<PathBuf, String> {
    let app_data_candidate = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to resolve app data directory: {}", e))?
        .join("models")
        .join("kokoro");
    if app_data_candidate.is_dir() {
        return Ok(app_data_candidate);
    }

    let resource_candidate = app_handle
        .path()
        .resolve("models/kokoro", BaseDirectory::Resource)
        .map_err(|e| format!("Failed to resolve app resource directory: {}", e))?;
    if resource_candidate.is_dir() {
        return Ok(resource_candidate);
    }

    Err(
        "Kokoro model directory not found. Expected <AppData>/models/kokoro or bundled resources/models/kokoro."
            .to_string(),
    )
}

fn open_output_stream(app_handle: &AppHandle) -> std::result::Result<MixerDeviceSink, String> {
    let settings = get_settings(app_handle);
    let selected_name = settings
        .selected_output_device
        .clone()
        .unwrap_or_else(|| "default".to_string());

    if selected_name != "default" {
        let host = cpal::default_host();
        if let Ok(devices) = host.output_devices() {
            for device in devices {
                let Ok(desc) = device.description() else {
                    continue;
                };
                if desc.name() == selected_name {
                    return DeviceSinkBuilder::from_device(device)
                        .and_then(|builder| builder.open_stream())
                        .map_err(|e| format!("Failed to open selected output device: {}", e));
                }
            }
            error!(
                "Selected output device '{}' not found. Falling back to default.",
                selected_name
            );
        }
    }

    DeviceSinkBuilder::from_default_device()
        .and_then(|builder| builder.open_stream())
        .map_err(|e| format!("Failed to open default audio output: {}", e))
}

/// Soft target for the first chunk.  The chunker stops adding sentences once
/// it exceeds this limit.  Kept small so chunk-1 synthesis finishes quickly
/// (~1–2 s at typical RTF 0.50), giving near-instant playback start.
///
/// Importantly this is a *sentence-level* target: a single sentence that is
/// longer than this value is never split mid-sentence (only the regular
/// `CHUNK_HARD_LIMIT_CHARS` can force a mid-sentence split).  This avoids
/// audible prosody breaks within a sentence.
const FIRST_CHUNK_TARGET_CHARS: usize = 60;
const CHUNK_TARGET_CHARS: usize = 260;
const CHUNK_HARD_LIMIT_CHARS: usize = 320;
const SENTENCE_JOIN_SOFT_LIMIT_CHARS: usize = 380;

fn split_text_for_playback(text: &str, shorten_first_chunk: bool) -> Vec<String> {
    let normalized = normalize_text_whitespace(text);
    if normalized.is_empty() {
        return vec![];
    }

    let sentences = split_into_sentences(&normalized);
    let mut chunks: Vec<String> = Vec::new();
    let mut current_chunk = String::new();
    let mut is_first_chunk = true;

    for sentence in sentences {
        let sentence_parts =
            split_long_segment(sentence, CHUNK_TARGET_CHARS, CHUNK_HARD_LIMIT_CHARS);

        for part in sentence_parts {
            if current_chunk.is_empty() {
                current_chunk.push_str(part);
                continue;
            }

            let current_target = if is_first_chunk && shorten_first_chunk {
                FIRST_CHUNK_TARGET_CHARS
            } else {
                CHUNK_TARGET_CHARS
            };

            let candidate_len = current_chunk.len() + 1 + part.len();
            if candidate_len <= current_target
                || (candidate_len <= SENTENCE_JOIN_SOFT_LIMIT_CHARS
                    && current_chunk.len() < current_target / 2)
            {
                current_chunk.push(' ');
                current_chunk.push_str(part);
                continue;
            }

            // Flush current_chunk.  For the first chunk, if the single sentence
            // inside it is too long for fast synthesis, split it at a natural
            // clause boundary (comma, semicolon, colon, closing paren/bracket).
            // This avoids the jarring mid-word prosody breaks that arbitrary
            // splits cause, while still keeping time-to-first-audio low.
            if is_first_chunk
                && shorten_first_chunk
                && current_chunk.len() > FIRST_CHUNK_TARGET_CHARS
            {
                let (head, tail) =
                    split_at_clause_boundary(&current_chunk, FIRST_CHUNK_TARGET_CHARS);
                chunks.push(head);
                if let Some(remainder) = tail {
                    chunks.push(remainder);
                }
            } else {
                chunks.push(current_chunk.trim().to_string());
            }
            is_first_chunk = false;
            current_chunk.clear();
            current_chunk.push_str(part);
        }
    }

    // Flush the last accumulated chunk.
    let trailing = current_chunk.trim();
    if !trailing.is_empty() {
        if is_first_chunk && shorten_first_chunk && trailing.len() > FIRST_CHUNK_TARGET_CHARS {
            let (head, tail) = split_at_clause_boundary(trailing, FIRST_CHUNK_TARGET_CHARS);
            chunks.push(head);
            if let Some(remainder) = tail {
                chunks.push(remainder);
            }
        } else {
            chunks.push(trailing.to_string());
        }
    }

    if chunks.is_empty() {
        vec![normalized]
    } else {
        chunks
    }
}

/// Split text at the best natural boundary near `target` chars.
///
/// Scans the window `[target/2 .. target*2]` for sentence-ending punctuation
/// (`.!?`), clause boundaries (`,;:`), and closing brackets/parens (`)]`).
/// Among all candidates the one closest to `target` wins — this keeps the
/// first chunk near the ideal size without favouring an awkward split just
/// because it is a comma rather than a closing paren.
///
/// If no natural boundary is found, returns the whole text unsplit — a longer
/// first chunk is preferable to a jarring mid-word break.
fn split_at_clause_boundary(text: &str, target: usize) -> (String, Option<String>) {
    let search_limit = (target * 2).min(text.len());
    let min_pos = target / 2;

    // (position, effective_distance) — lower distance wins.
    // Sentence-end and closing-bracket boundaries get a bonus (reduced
    // effective distance) so that a slightly farther `.` or `)` is preferred
    // over a nearby `,` when both are close to `target`.  This avoids
    // splitting inside parentheticals or between "August 19," and "1946)".
    const STRONG_BOUNDARY_BONUS: usize = 10;
    let mut best: Option<(usize, usize)> = None; // (pos, effective_distance)

    for (idx, ch) in text.char_indices() {
        let pos = idx + ch.len_utf8();
        if pos < min_pos {
            continue;
        }
        if idx > search_limit {
            break;
        }

        let ch_len = ch.len_utf8();
        if is_punctuation_between_ascii_digits(text, idx, ch_len) {
            continue;
        }

        let is_strong = is_sentence_boundary_char(ch) || matches!(ch, ')' | ']');
        let is_weak = is_clause_boundary_char(ch);

        if is_strong || is_weak {
            let raw_dist = pos.abs_diff(target);
            let eff_dist = if is_strong {
                raw_dist.saturating_sub(STRONG_BOUNDARY_BONUS)
            } else {
                raw_dist
            };
            if best.is_none() || eff_dist < best.unwrap().1 {
                best = Some((pos, eff_dist));
            }
        }
    }

    match best {
        Some((pos, _)) if pos < text.len() => {
            let head = text[..pos].trim().to_string();
            let tail = text[pos..].trim().to_string();
            if tail.is_empty() {
                (head, None)
            } else {
                (head, Some(tail))
            }
        }
        _ => {
            // No good boundary found — keep the text whole rather than
            // introducing a mid-word break.
            (text.trim().to_string(), None)
        }
    }
}

fn normalize_text_whitespace(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut in_whitespace = false;

    for ch in text.chars() {
        if ch.is_whitespace() {
            if !in_whitespace {
                normalized.push(' ');
                in_whitespace = true;
            }
        } else {
            normalized.push(ch);
            in_whitespace = false;
        }
    }

    normalized.trim().to_string()
}

fn split_into_sentences(text: &str) -> Vec<&str> {
    let mut sentences = Vec::new();
    let mut start = 0usize;
    let bytes = text.as_bytes();

    for (idx, ch) in text.char_indices() {
        if !is_sentence_boundary_char(ch) {
            continue;
        }

        let ch_len = ch.len_utf8();
        if is_punctuation_between_ascii_digits(text, idx, ch_len) {
            continue;
        }

        let next_idx = idx + ch_len;
        let next_is_boundary = next_idx >= bytes.len()
            || bytes[next_idx].is_ascii_whitespace()
            || matches!(bytes[next_idx], b'"' | b'\'' | b')' | b']');
        if !next_is_boundary {
            continue;
        }

        let segment = text[start..next_idx].trim();
        if !segment.is_empty() {
            sentences.push(segment);
        }
        start = next_idx;
    }

    if start < text.len() {
        let trailing = text[start..].trim();
        if !trailing.is_empty() {
            sentences.push(trailing);
        }
    }

    if sentences.is_empty() {
        vec![text]
    } else {
        sentences
    }
}

fn split_long_segment(segment: &str, target: usize, hard_limit: usize) -> Vec<&str> {
    if segment.len() <= hard_limit {
        return vec![segment];
    }

    let mut parts = Vec::new();
    let mut offset = 0usize;
    while offset < segment.len() {
        let remaining = &segment[offset..];
        if remaining.len() <= hard_limit {
            parts.push(remaining.trim());
            break;
        }

        let boundary = find_split_index(remaining, target, hard_limit);
        let piece = remaining[..boundary].trim();
        if !piece.is_empty() {
            parts.push(piece);
        }
        offset += boundary;
    }

    parts
}

fn find_split_index(text: &str, target: usize, hard_limit: usize) -> usize {
    let mut last_sentence = None;
    let mut last_clause = None;
    let mut last_space = None;

    for (idx, ch) in text.char_indices() {
        if idx > hard_limit {
            break;
        }
        let ch_len = ch.len_utf8();
        let is_numeric_punctuation = is_punctuation_between_ascii_digits(text, idx, ch_len);
        if ch.is_whitespace() {
            last_space = Some(idx);
        }
        if is_sentence_boundary_char(ch) {
            if !is_numeric_punctuation {
                last_sentence = Some(idx + ch_len);
            }
        } else if is_clause_boundary_char(ch) && !is_numeric_punctuation {
            last_clause = Some(idx + ch_len);
        }
    }

    if let Some(idx) = last_sentence.filter(|idx| *idx >= target / 2 && *idx <= hard_limit) {
        return idx;
    }
    if let Some(idx) = last_clause.filter(|idx| *idx >= target / 2 && *idx <= hard_limit) {
        return idx;
    }
    if let Some(idx) = last_space.filter(|idx| *idx >= target / 2 && *idx <= hard_limit) {
        return idx;
    }

    let fallback = text
        .char_indices()
        .find(|(idx, _)| *idx >= hard_limit)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len());

    adjust_split_index_for_numeric_connector(text, fallback)
}

fn is_sentence_boundary_char(ch: char) -> bool {
    matches!(ch, '.' | '!' | '?' | '…')
}

fn is_clause_boundary_char(ch: char) -> bool {
    matches!(ch, ',' | ';' | ':')
}

fn is_punctuation_between_ascii_digits(text: &str, idx: usize, ch_len: usize) -> bool {
    let ch = text[idx..idx + ch_len].chars().next();
    if !matches!(ch, Some('.' | ',')) {
        return false;
    }

    let prev = text[..idx].chars().next_back();
    let next = text[idx + ch_len..].chars().next();

    matches!(
        (prev, next),
        (Some(left), Some(right)) if left.is_ascii_digit() && right.is_ascii_digit()
    )
}

fn adjust_split_index_for_numeric_connector(text: &str, idx: usize) -> usize {
    if idx == 0 || idx >= text.len() || !split_breaks_numeric_connector(text, idx) {
        return idx;
    }

    let mut forward = idx;
    while forward < text.len() && split_breaks_numeric_connector(text, forward) {
        let Some(next_ch) = text[forward..].chars().next() else {
            break;
        };
        forward += next_ch.len_utf8();
    }
    if !split_breaks_numeric_connector(text, forward) {
        return forward;
    }

    let mut backward = idx;
    while backward > 0 && split_breaks_numeric_connector(text, backward) {
        let Some((prev_idx, _)) = text[..backward].char_indices().next_back() else {
            break;
        };
        backward = prev_idx;
    }
    backward
}

fn split_breaks_numeric_connector(text: &str, idx: usize) -> bool {
    if idx == 0 || idx >= text.len() {
        return false;
    }

    let prev = text[..idx].chars().next_back();
    let next = text[idx..].chars().next();

    // Split between "2" and ".0"
    let next_breaks = matches!(next, Some('.' | ','))
        && prev.map(|c| c.is_ascii_digit()).unwrap_or(false)
        && text[idx + next.unwrap().len_utf8()..]
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false);

    if next_breaks {
        return true;
    }

    // Split between "2." and "0"
    matches!(prev, Some('.' | ','))
        && next.map(|c| c.is_ascii_digit()).unwrap_or(false)
        && text[..idx - prev.unwrap().len_utf8()]
            .chars()
            .next_back()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{
        estimate_kokoro_style_index, find_split_index, infer_kokoro_tuning_for_cpu_count,
        normalize_kokoro_language_code, request_is_active, select_kokoro_voice_for_text,
        select_voice_for_language, split_into_sentences, split_text_for_playback,
        take_engine_for_active_request, MAX_PARALLEL_SYNTH_ENGINES,
    };
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn keeps_sentence_boundaries_for_short_text() {
        let chunks = split_text_for_playback("First sentence. Second sentence?", true);
        assert_eq!(chunks, vec!["First sentence. Second sentence?"]);
    }

    #[test]
    fn splits_long_text_without_empty_chunks() {
        let text = "This is a long sentence with several clauses, and it keeps going so we can force a split point near punctuation. \
            Here is another sentence with enough additional words to exceed the hard limit while still keeping punctuation intact.";
        let chunks = split_text_for_playback(&text.repeat(8), true);
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|chunk| !chunk.trim().is_empty()));
    }

    #[test]
    fn keeps_decimal_numbers_in_sentence_splitting() {
        let sentences = split_into_sentences("Version 2.0 is out. Install now.");
        assert_eq!(sentences, vec!["Version 2.0 is out.", "Install now."]);
    }

    #[test]
    fn does_not_use_decimal_or_comma_between_digits_as_chunk_boundary() {
        let decimal_text = format!("{}2.0 {}", "word ".repeat(40), "word ".repeat(40));
        let decimal_chunks = split_text_for_playback(&decimal_text, true);
        for pair in decimal_chunks.windows(2) {
            assert!(!(pair[0].trim_end().ends_with("2.") && pair[1].trim_start().starts_with('0')));
        }

        let comma_text = format!("{}1,000 {}", "word ".repeat(40), "word ".repeat(40));
        let comma_chunks = split_text_for_playback(&comma_text, true);
        for pair in comma_chunks.windows(2) {
            assert!(
                !(pair[0].trim_end().ends_with("1,") && pair[1].trim_start().starts_with("000"))
            );
        }
    }

    #[test]
    fn fallback_split_does_not_cut_inside_decimal_token() {
        let text = "prefixv2.0suffix";
        let dot_idx = text.find('.').expect("decimal point should exist");
        let split_idx = find_split_index(text, dot_idx, dot_idx);
        assert_eq!(split_idx, dot_idx + 2);
    }

    #[test]
    fn split_index_skips_numeric_decimal_boundary() {
        let text = "alpha beta gamma 2.0 delta epsilon";
        let decimal_boundary = text.find('.').expect("decimal point should exist") + 1;
        let split_idx = find_split_index(text, 12, decimal_boundary);
        assert_ne!(split_idx, decimal_boundary);
    }

    #[test]
    fn first_chunk_splits_at_clause_boundary_not_mid_word() {
        // First sentence is 171 chars — well over FIRST_CHUNK_TARGET_CHARS (60).
        // The chunker should split at the closing paren (a strong boundary)
        // rather than at a mid-sentence word boundary.
        let text = "William Jefferson Clinton (né Blythe III; born August 19, 1946) \
            is an American politician and lawyer who served as the 42nd president \
            of the United States from 1993 to 2001. A member of the Democratic \
            Party, he previously served as the attorney general of Arkansas.";
        let chunks = split_text_for_playback(text, true);
        assert!(chunks.len() >= 2, "should split into multiple chunks");
        // First chunk must end at the closing paren — a natural clause boundary.
        assert!(
            chunks[0].ends_with(')') || chunks[0].ends_with('.'),
            "first chunk should end at a natural boundary, got: {:?}",
            chunks[0]
        );
        // First chunk should be well under 100 chars for fast synthesis.
        assert!(
            chunks[0].len() <= 100,
            "first chunk should be ≤100 chars for fast synthesis, got {} chars",
            chunks[0].len()
        );
    }

    #[test]
    fn kokoro_tuning_uses_single_worker_on_constrained_cpu() {
        let tuning = infer_kokoro_tuning_for_cpu_count(2);
        assert_eq!(tuning.target_workers, 1);
        assert!(tuning.threads_per_worker >= 1);
    }

    #[test]
    fn kokoro_tuning_scales_workers_on_larger_cpu() {
        let tuning = infer_kokoro_tuning_for_cpu_count(8);
        // 8 CPUs → 7 compute budget → min(MAX_PARALLEL_SYNTH_ENGINES, 7) = 2 workers
        assert_eq!(tuning.target_workers, MAX_PARALLEL_SYNTH_ENGINES);
        assert!(tuning.threads_per_worker >= 1);
    }

    #[test]
    fn style_index_estimate_is_non_zero() {
        let chunks = vec!["Hello world.".to_string(), "How are you?".to_string()];
        assert!(estimate_kokoro_style_index(&chunks) > 0);
    }

    #[test]
    fn selects_french_voice_when_available() {
        let voices = vec![
            "af_heart".to_string(),
            "ff_siwis".to_string(),
            "bf_emma".to_string(),
        ];
        let selected = select_kokoro_voice_for_text(Some("fr"), None, &voices);
        assert_eq!(selected, "ff_siwis");
    }

    #[test]
    fn normalizes_all_kokoro_supported_languages() {
        assert_eq!(normalize_kokoro_language_code(Some("en-US")), Some("en-us"));
        assert_eq!(normalize_kokoro_language_code(Some("en-GB")), Some("en-gb"));
        assert_eq!(normalize_kokoro_language_code(Some("es-AR")), Some("es"));
        assert_eq!(normalize_kokoro_language_code(Some("fr-CA")), Some("fr"));
        assert_eq!(normalize_kokoro_language_code(Some("hi-IN")), Some("hi"));
        assert_eq!(normalize_kokoro_language_code(Some("it-IT")), Some("it"));
        assert_eq!(normalize_kokoro_language_code(Some("ja-JP")), Some("ja"));
        assert_eq!(normalize_kokoro_language_code(Some("pt-PT")), Some("pt-br"));
        assert_eq!(normalize_kokoro_language_code(Some("zh-Hant")), Some("cmn"));
        assert_eq!(normalize_kokoro_language_code(Some("zh-CN")), Some("cmn"));
        assert_eq!(normalize_kokoro_language_code(Some("yue")), Some("cmn"));
        assert_eq!(normalize_kokoro_language_code(Some("cmn")), Some("cmn"));
    }

    #[test]
    fn selects_voice_for_each_supported_kokoro_language() {
        let voices = vec![
            "af_heart".to_string(),
            "bf_emma".to_string(),
            "ef_dora".to_string(),
            "ff_siwis".to_string(),
            "hf_beta".to_string(),
            "if_alpha".to_string(),
            "jf_alpha".to_string(),
            "pf_dora".to_string(),
            "zf_xiaobei".to_string(),
        ];

        assert!(select_voice_for_language("en-us", &voices).starts_with("af_"));
        assert!(select_voice_for_language("en-gb", &voices).starts_with("bf_"));
        assert!(select_voice_for_language("es", &voices).starts_with("ef_"));
        assert!(select_voice_for_language("fr", &voices).starts_with("ff_"));
        assert!(select_voice_for_language("hi", &voices).starts_with("hf_"));
        assert!(select_voice_for_language("it", &voices).starts_with("if_"));
        assert!(select_voice_for_language("ja", &voices).starts_with("jf_"));
        assert!(select_voice_for_language("pt-br", &voices).starts_with("pf_"));
        assert!(select_voice_for_language("cmn", &voices).starts_with("zf_"));
    }

    #[test]
    fn selected_voice_override_has_priority() {
        let voices = vec![
            "af_heart".to_string(),
            "ff_siwis".to_string(),
            "jf_alpha".to_string(),
        ];

        let selected = select_kokoro_voice_for_text(Some("fr"), Some("jf_alpha"), &voices);
        assert_eq!(selected, "jf_alpha");
    }

    #[test]
    fn selected_voice_override_falls_back_when_unavailable() {
        let voices = vec!["af_heart".to_string(), "ff_siwis".to_string()];
        let selected = select_kokoro_voice_for_text(Some("fr"), Some("jf_alpha"), &voices);
        assert_eq!(selected, "ff_siwis");
    }

    #[test]
    fn request_active_check_requires_matching_generation_and_request() {
        let generation = Arc::new(AtomicU64::new(11));
        let active_request = Arc::new(AtomicU64::new(11));
        assert!(request_is_active(&generation, &active_request, 11));

        generation.store(12, Ordering::SeqCst);
        assert!(!request_is_active(&generation, &active_request, 11));

        generation.store(11, Ordering::SeqCst);
        active_request.store(0, Ordering::SeqCst);
        assert!(!request_is_active(&generation, &active_request, 11));
    }

    #[test]
    fn engine_take_aborts_when_request_is_cancelled() {
        let engine_slot = Arc::new(Mutex::new(None::<tts_rs::engines::kokoro::KokoroEngine>));
        let generation = Arc::new(AtomicU64::new(5));
        let active_request = Arc::new(AtomicU64::new(5));

        // Slot is None, so take_engine will keep polling.
        // Cancel the request so it gives up.
        let engine_slot_for_worker = Arc::clone(&engine_slot);
        let generation_for_worker = Arc::clone(&generation);
        let active_request_for_worker = Arc::clone(&active_request);
        let worker = thread::spawn(move || {
            take_engine_for_active_request(
                &engine_slot_for_worker,
                &generation_for_worker,
                &active_request_for_worker,
                5,
            )
            .is_none()
        });

        thread::sleep(Duration::from_millis(10));
        generation.store(6, Ordering::SeqCst);
        active_request.store(0, Ordering::SeqCst);

        assert!(
            worker.join().expect("worker thread should join"),
            "take_engine should abort when request is cancelled"
        );
    }
}
