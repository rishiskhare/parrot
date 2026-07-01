//! # transcribe-rs
//!
//! A Rust library providing text-to-speech synthesis using the Kokoro engine.
//!
//! ## Features
//!
//! - **Kokoro TTS**: High-quality text-to-speech with multiple voices and languages
//! - **Flexible Model Loading**: Load models with custom parameters
//! - **Multiple Voices**: Support for 9 languages with various voice styles
//!
//! ## Quick Start
//!
//! ```toml
//! [dependencies]
//! transcribe-rs = { version = "0.2", features = ["kokoro"] }
//! ```
//!
//! ```ignore
//! use std::path::PathBuf;
//! use transcribe_rs::{engines::kokoro::KokoroEngine, SynthesisEngine};
//!
//! let mut engine = KokoroEngine::new();
//! engine.load_model(&PathBuf::from("models/kokoro-v1.0"))?;
//!
//! let result = engine.synthesize("Hello, world!", None)?;
//! result.write_wav(&PathBuf::from("output.wav"))?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod engines;

use std::path::Path;

/// The result of a synthesis (text-to-speech) operation.
///
/// Contains raw f32 audio samples and the sample rate of the output audio.
#[derive(Debug)]
pub struct SynthesisResult {
    /// Raw audio samples as f32 values
    pub samples: Vec<f32>,
    /// Sample rate of the audio (24000 for Kokoro)
    pub sample_rate: u32,
}

impl SynthesisResult {
    /// Write the audio to a 32-bit float WAV file.
    pub fn write_wav(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: self.sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(path, spec)?;
        for &sample in &self.samples {
            writer.write_sample(sample)?;
        }
        writer.finalize()?;
        Ok(())
    }

    /// Duration of the audio in seconds.
    pub fn duration_secs(&self) -> f64 {
        self.samples.len() as f64 / self.sample_rate as f64
    }
}

/// Common interface for text-to-speech synthesis engines.
///
/// This trait defines the standard operations that all synthesis engines must support.
/// Each engine may have different parameter types for model loading and inference configuration.
pub trait SynthesisEngine {
    /// Parameters for configuring inference behavior (voice, speed, etc.)
    type SynthesisParams;
    /// Parameters for configuring model loading (threads, etc.)
    type ModelParams: Default;

    /// Load a model from the specified path using default parameters.
    fn load_model(&mut self, model_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        self.load_model_with_params(model_path, Self::ModelParams::default())
    }

    /// Load a model from the specified path with custom parameters.
    fn load_model_with_params(
        &mut self,
        model_path: &Path,
        params: Self::ModelParams,
    ) -> Result<(), Box<dyn std::error::Error>>;

    /// Unload the currently loaded model and free associated resources.
    fn unload_model(&mut self);

    /// Synthesize speech from the given text.
    fn synthesize(
        &mut self,
        text: &str,
        params: Option<Self::SynthesisParams>,
    ) -> Result<SynthesisResult, Box<dyn std::error::Error>>;

    /// Synthesize speech from the given text and write to a WAV file.
    ///
    /// Default implementation calls `synthesize()` then `SynthesisResult::write_wav()`.
    fn synthesize_to_file(
        &mut self,
        text: &str,
        wav_path: &Path,
        params: Option<Self::SynthesisParams>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.synthesize(text, params)?.write_wav(wav_path)
    }
}
