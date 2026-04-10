use std::path::{Path, PathBuf};

use crate::{SynthesisEngine, SynthesisResult};

use super::model::{KokoroError, KokoroModel, SAMPLE_RATE};
use super::phonemizer::EspeakConfig;

/// Parameters for configuring Kokoro model loading.
#[derive(Debug, Clone, Default)]
pub struct KokoroModelParams {
    /// Number of CPU threads to use for inference.
    /// `None` uses the ORT default (typically all available cores).
    pub num_threads: Option<usize>,
    /// Path for caching the Level3-optimized ONNX graph.
    ///
    /// - First load: ORT runs Level3 optimization and serialises the result here.
    /// - Subsequent loads: the pre-built graph is loaded at `Disable` optimization,
    ///   skipping the expensive 5–10 s re-optimization step entirely.
    ///
    /// Always write to a writable location (e.g. app data dir); bundled resource
    /// directories may be read-only.
    pub optimized_model_cache_path: Option<PathBuf>,
}

/// Parameters for configuring a Kokoro synthesis request.
#[derive(Debug, Clone)]
pub struct KokoroInferenceParams {
    /// Voice name (e.g. `"af_heart"`, `"bf_emma"`, `"jf_alpha"`).
    pub voice: String,
    /// Speech speed multiplier. Range: 0.5–2.0, default 1.0.
    pub speed: f32,
    /// Override the style vector index. `None` = auto (uses phoneme token count).
    pub style_index: Option<usize>,
}

impl Default for KokoroInferenceParams {
    fn default() -> Self {
        Self {
            voice: "af_heart".to_string(),
            speed: 1.0,
            style_index: None,
        }
    }
}

/// Kokoro text-to-speech engine.
///
/// Uses the Kokoro-82M ONNX model for high-quality, fast TTS with support
/// for 9 languages. Requires espeak-ng for phonemization.
///
/// # Quick Start
///
/// ```rust,no_run
/// use tts_rs::{SynthesisEngine, engines::kokoro::KokoroEngine};
/// use std::path::PathBuf;
///
/// // Uses system espeak-ng from PATH
/// let mut engine = KokoroEngine::new();
/// engine.load_model(&PathBuf::from("models/kokoro"))?;
/// let result = engine.synthesize("Hello, world!", None)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Bundled espeak-ng
///
/// ```rust,no_run
/// use tts_rs::engines::kokoro::KokoroEngine;
/// use std::path::PathBuf;
///
/// // Point to a bundled espeak-ng binary and data directory
/// let engine = KokoroEngine::with_espeak(
///     Some(PathBuf::from("/app/resources/espeak-ng/espeak-ng")),
///     Some(PathBuf::from("/app/resources/espeak-ng-data")),
/// );
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct KokoroEngine {
    model: Option<KokoroModel>,
    model_path: Option<PathBuf>,
    espeak: EspeakConfig,
}

impl Default for KokoroEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl KokoroEngine {
    /// Create a new engine that uses `espeak-ng` from PATH.
    pub fn new() -> Self {
        Self {
            model: None,
            model_path: None,
            espeak: EspeakConfig::default(),
        }
    }

    /// Create a new engine with explicit espeak-ng binary and data paths.
    ///
    /// Use this when bundling espeak-ng with your application. Either path
    /// can be `None` to fall back to the system default.
    pub fn with_espeak(
        bin_path: Option<PathBuf>,
        data_path: Option<PathBuf>,
    ) -> Self {
        Self {
            model: None,
            model_path: None,
            espeak: EspeakConfig { bin_path, data_path },
        }
    }

    /// List all available voice names (requires model to be loaded).
    pub fn list_voices(&self) -> Vec<&str> {
        self.model
            .as_ref()
            .map(|m| m.list_voices())
            .unwrap_or_default()
    }
}

impl Drop for KokoroEngine {
    fn drop(&mut self) {
        self.unload_model();
    }
}

impl SynthesisEngine for KokoroEngine {
    type SynthesisParams = KokoroInferenceParams;
    type ModelParams = KokoroModelParams;

    fn load_model_with_params(
        &mut self,
        model_path: &Path,
        params: Self::ModelParams,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let model = KokoroModel::load(
            model_path,
            params.num_threads,
            params.optimized_model_cache_path.as_deref(),
        )?;
        self.model = Some(model);
        self.model_path = Some(model_path.to_path_buf());
        Ok(())
    }

    fn unload_model(&mut self) {
        self.model = None;
        self.model_path = None;
    }

    fn synthesize(
        &mut self,
        text: &str,
        params: Option<Self::SynthesisParams>,
    ) -> Result<SynthesisResult, Box<dyn std::error::Error>> {
        let model = self.model.as_mut().ok_or(KokoroError::ModelNotLoaded)?;

        let p = params.unwrap_or_default();
        let samples =
            model.synthesize_text(text, &p.voice, p.speed, p.style_index, &self.espeak)?;

        Ok(SynthesisResult {
            samples,
            sample_rate: SAMPLE_RATE,
        })
    }
}
