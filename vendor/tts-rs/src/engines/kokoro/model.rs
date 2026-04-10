use std::collections::HashMap;
use std::path::Path;

use ndarray::Array2;
use ort::execution_providers::CPUExecutionProvider;
use ort::inputs;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::TensorRef;

use super::phonemizer::{phonemize, voice_lang, EspeakConfig};
use super::voices::VoiceStore;

/// Maximum number of phoneme tokens per chunk (before padding).
pub const MAX_PHONEME_LEN: usize = 510;

/// Style vector dimension for Kokoro.
pub const STYLE_DIM: usize = 256;

/// Output sample rate from the Kokoro model.
pub const SAMPLE_RATE: u32 = 24000;

/// Crossfade (in samples) used when concatenating chunk audio.
const CHUNK_CROSSFADE_SAMPLES: usize = 240; // 10ms @ 24kHz

#[derive(thiserror::Error, Debug)]
pub enum KokoroError {
    #[error("ONNX runtime error: {0}")]
    Ort(#[from] ort::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Array shape error: {0}")]
    Shape(#[from] ndarray::ShapeError),
    #[error(
        "espeak-ng not found. Install: Linux: `sudo apt-get install espeak-ng`, \
         macOS: `brew install espeak-ng`, Windows: https://espeak-ng.org/download"
    )]
    EspeakNotFound,
    #[error("Phonemization failed: {0}")]
    PhonemizerFailed(String),
    #[error("Voice '{0}' not found. Call list_voices() to see available voices.")]
    VoiceNotFound(String),
    #[error("Model not loaded. Call load_model() first.")]
    ModelNotLoaded,
    #[error("Invalid config.json: {0}")]
    Config(String),
    #[error("Failed to parse voice file: {0}")]
    VoiceParse(String),
}

/// Internal Kokoro ONNX model state.
pub struct KokoroModel {
    session: Session,
    voice_store: VoiceStore,
    vocab: HashMap<char, i64>,
    /// Detected input name: "input_ids" or "tokens"
    tokens_input_name: String,
    /// True if the speed input expects int32, false for float32
    speed_is_int32: bool,
}

impl KokoroModel {
    /// Load the Kokoro model from a directory.
    ///
    /// The directory must contain:
    /// - An `.onnx` file (preferably `kokoro-quant-convinteger.onnx`)
    /// - A `voices-v1.0.bin` voice archive
    /// - Optionally a `config.json` for vocabulary (falls back to hardcoded)
    pub fn load(
        model_dir: &Path,
        num_threads: Option<usize>,
        optimized_cache_path: Option<&Path>,
    ) -> Result<Self, KokoroError> {
        let onnx_path = find_onnx_file(model_dir)?;
        log::info!("Loading Kokoro model from {}", onnx_path.display());

        let session = init_session(&onnx_path, num_threads, optimized_cache_path)?;

        // Detect input names at load time
        let tokens_input_name = detect_tokens_input(&session);
        let speed_is_int32 = detect_speed_type(&session);

        log::info!(
            "Detected: tokens_input='{}', speed_is_int32={}",
            tokens_input_name,
            speed_is_int32
        );

        // Load voices
        let voices_path = model_dir.join("voices-v1.0.bin");
        if !voices_path.exists() {
            return Err(KokoroError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "Voice file not found at {}. Download it from the Kokoro model repository.",
                    voices_path.display()
                ),
            )));
        }
        let voice_store = VoiceStore::load(&voices_path)?;

        // Load vocabulary
        let config_path = model_dir.join("config.json");
        let vocab = if config_path.exists() {
            log::info!("Loading vocab from config.json");
            super::vocab::load_vocab(&config_path)?
        } else {
            log::warn!("config.json not found, using hardcoded vocab");
            super::vocab::hardcoded_vocab()
        };

        Ok(Self {
            session,
            voice_store,
            vocab,
            tokens_input_name,
            speed_is_int32,
        })
    }

    /// Synthesize audio from text using the given voice and speed.
    pub fn synthesize_text(
        &mut self,
        text: &str,
        voice_name: &str,
        speed: f32,
        style_idx_override: Option<usize>,
        espeak: &EspeakConfig,
    ) -> Result<Vec<f32>, KokoroError> {
        let lang = voice_lang(voice_name);
        let ids = phonemize(text, lang, &self.vocab, espeak)?;

        if ids.is_empty() {
            log::warn!("No phoneme tokens produced for text: {text:?}");
            return Ok(vec![]);
        }

        // Split into chunks if needed. Keep a stable style index so adjacent chunks
        // don't change style/prosody based on chunk length.
        let style_idx = style_idx_override.unwrap_or(ids.len());
        let estimated_samples = ids.len() * 300;
        let chunks = if ids.len() > MAX_PHONEME_LEN {
            log::debug!(
                "Kokoro phoneme sequence exceeded limit ({} > {}), chunking",
                ids.len(),
                MAX_PHONEME_LEN
            );
            split_chunks(&ids)
        } else {
            vec![ids]
        };

        let mut combined = Vec::with_capacity(estimated_samples);

        for chunk_ids in chunks.iter() {
            let style = self.voice_store.get_style(voice_name, style_idx)?;
            let audio = self.synthesize_chunk(chunk_ids, &style, speed)?;
            if audio.is_empty() {
                continue;
            }

            if combined.is_empty() {
                combined.extend_from_slice(&audio);
            } else {
                append_with_crossfade(&mut combined, &audio, CHUNK_CROSSFADE_SAMPLES);
            }
        }

        Ok(combined)
    }

    /// Run ONNX inference on a single chunk of phoneme token IDs.
    fn synthesize_chunk(
        &mut self,
        tokens: &[i64],
        style: &[f32; STYLE_DIM],
        speed: f32,
    ) -> Result<Vec<f32>, KokoroError> {
        let seq_len = tokens.len() + 2; // +2 for padding tokens

        // Build tokens tensor: [[0, t1..tN, 0]]
        let mut padded = vec![0i64; seq_len];
        padded[1..seq_len - 1].copy_from_slice(tokens);
        let tokens_arr = Array2::from_shape_vec((1, seq_len), padded)?;

        // Build style tensor: [[s0..s255]] — use a view to avoid copying the 256-float array
        let style_view = ndarray::ArrayView2::from_shape((1, STYLE_DIM), style.as_slice())?;

        // Run session
        let output = if self.speed_is_int32 {
            let speed_arr = ndarray::arr1(&[speed as i32]);
            let inputs = inputs![
                self.tokens_input_name.as_str() => TensorRef::from_array_view(tokens_arr.view())?,
                "style" => TensorRef::from_array_view(style_view)?,
                "speed" => TensorRef::from_array_view(speed_arr.view())?,
            ];
            self.session.run(inputs)?
        } else {
            let speed_arr = ndarray::arr1(&[speed]);
            let inputs = inputs![
                self.tokens_input_name.as_str() => TensorRef::from_array_view(tokens_arr.view())?,
                "style" => TensorRef::from_array_view(style_view)?,
                "speed" => TensorRef::from_array_view(speed_arr.view())?,
            ];
            self.session.run(inputs)?
        };

        // Extract first output as waveform
        let first_output = output
            .iter()
            .next()
            .ok_or_else(|| KokoroError::Ort(ort::Error::new("No output from model")))?;
        let waveform = first_output.1.try_extract_array::<f32>()?;

        Ok(waveform.as_slice().unwrap_or(&[]).to_vec())
    }

    /// List all available voice names.
    pub fn list_voices(&self) -> Vec<&str> {
        self.voice_store.list_voices()
    }
}

/// Find the ONNX model file in the given directory.
///
/// Prefers `kokoro-quant-convinteger.onnx`, then falls back to the first `.onnx` file found.
fn find_onnx_file(model_dir: &Path) -> Result<std::path::PathBuf, KokoroError> {
    let preferred = model_dir.join("kokoro-quant-convinteger.onnx");
    if preferred.exists() {
        return Ok(preferred);
    }

    // Scan for any .onnx file
    for entry in std::fs::read_dir(model_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("onnx") {
            log::info!("Using ONNX file: {}", path.display());
            return Ok(path);
        }
    }

    Err(KokoroError::Io(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("No .onnx file found in {}", model_dir.display()),
    )))
}

/// Initialize an ONNX session with optional on-disk graph caching.
///
/// The first time a model is loaded, ORT runs Level3 graph optimization (5–10 s)
/// and serialises the result to `optimized_cache_path`.  Every subsequent load
/// reads the pre-optimized file directly at `Disable` optimization level, cutting
/// cold-start time to under one second.
///
/// If `optimized_cache_path` is `None` the original behaviour (always Level3) is
/// preserved, which is useful for unit-testing or read-only deployments.
fn init_session(
    onnx_path: &Path,
    num_threads: Option<usize>,
    optimized_cache_path: Option<&Path>,
) -> Result<Session, KokoroError> {
    let providers = vec![CPUExecutionProvider::default().build()];

    // Choose load path and optimization level depending on cache state.
    let (load_path, opt_level, write_cache) = match optimized_cache_path {
        // Pre-optimized graph already on disk → load it directly, skip optimization.
        Some(cache) if cache.exists() => {
            log::info!(
                "Loading pre-optimized Kokoro graph ({:.1} MB) from {:?} — skipping Level3",
                cache
                    .metadata()
                    .map(|m| m.len() as f64 / 1_048_576.0)
                    .unwrap_or(0.0),
                cache
            );
            (cache, GraphOptimizationLevel::Disable, false)
        }
        // Cache path given but file does not exist yet → build + persist.
        Some(cache) => {
            log::info!(
                "First load: running Level3 optimization; saving graph to {:?}",
                cache
            );
            (onnx_path, GraphOptimizationLevel::Level3, true)
        }
        // No cache path → original behaviour.
        None => (onnx_path, GraphOptimizationLevel::Level3, false),
    };

    let mut builder = Session::builder()?
        .with_optimization_level(opt_level)?
        .with_execution_providers(providers)?
        .with_parallel_execution(true)?;

    if write_cache {
        // Serialise the optimized graph so the next launch can skip optimization.
        let cache = optimized_cache_path.unwrap();
        builder = builder.with_optimized_model_path(cache)?;
    }

    if let Some(threads) = num_threads {
        builder = builder
            .with_intra_threads(threads)?
            .with_inter_threads(threads)?;
    }

    Ok(builder.commit_from_file(load_path)?)
}

/// Detect the token input name ("input_ids" or "tokens") from session inputs.
fn detect_tokens_input(session: &Session) -> String {
    for input in session.inputs() {
        if input.name() == "input_ids" || input.name() == "tokens" {
            return input.name().to_string();
        }
    }
    // Default to "input_ids" if neither is found
    "input_ids".to_string()
}

/// Detect whether the speed input expects int32 (true) or float32 (false).
fn detect_speed_type(session: &Session) -> bool {
    for input in session.inputs() {
        if input.name() == "speed" {
            // Check the type description
            let type_str = format!("{:?}", input.dtype());
            return type_str.contains("Int32") || type_str.contains("int32");
        }
    }
    // Default: modern Kokoro models use int32
    true
}

/// Split phoneme IDs into chunks of at most `MAX_PHONEME_LEN`, preferring punctuation.
fn split_chunks(ids: &[i64]) -> Vec<Vec<i64>> {
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < ids.len() {
        let end = (start + MAX_PHONEME_LEN).min(ids.len());
        if end == ids.len() {
            chunks.push(ids[start..end].to_vec());
            break;
        }

        // Try to find a good split point (last punctuation before `end`).
        // Punctuation IDs (hardcoded vocab): ';':1 ':':2 ',':3 '.':4 '!':5 '?':6
        const PUNCT_IDS: &[i64] = &[1, 2, 3, 4, 5, 6];
        let split = ids[start..end]
            .iter()
            .enumerate()
            .rev()
            .find(|(_, &id)| PUNCT_IDS.contains(&id))
            .map(|(i, _)| start + i + 1)
            .unwrap_or(end);

        chunks.push(ids[start..split].to_vec());
        start = split;
    }

    chunks
}

fn append_with_crossfade(dst: &mut Vec<f32>, src: &[f32], crossfade_samples: usize) {
    let overlap = crossfade_samples.min(dst.len()).min(src.len());
    if overlap == 0 {
        dst.extend_from_slice(src);
        return;
    }

    let dst_start = dst.len() - overlap;
    for i in 0..overlap {
        let t = (i + 1) as f32 / (overlap as f32 + 1.0);
        let left = dst[dst_start + i] * (1.0 - t);
        let right = src[i] * t;
        dst[dst_start + i] = left + right;
    }

    dst.extend_from_slice(&src[overlap..]);
}
