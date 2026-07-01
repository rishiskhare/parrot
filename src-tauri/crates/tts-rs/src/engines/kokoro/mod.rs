//! Kokoro-82M text-to-speech engine implementation.
//!
//! This module provides a Kokoro-based synthesis engine that uses the
//! Kokoro-82M ONNX model for text-to-speech conversion. The engine uses
//! espeak-ng for phonemization and supports 9 languages.
//!
//! # System Requirements
//!
//! **espeak-ng** must be installed on your system:
//! - **Linux**: `sudo apt-get install espeak-ng`
//! - **macOS**: `brew install espeak-ng`
//! - **Windows**: Download installer from <https://espeak-ng.org/download>
//!
//! # Model Directory Layout
//!
//! ```text
//! models/kokoro/
//! ├── kokoro-quant-convinteger.onnx   # 8-bit quantized model (88MB, CPU-optimized)
//! └── voices-v1.0.bin                  # Voice data archive (.npz format)
//! ```
//!
//! Download links:
//! - Model: <https://github.com/taylorchu/kokoro-onnx/releases/tag/v0.2.0>
//! - Voices: <https://github.com/thewh1teagle/kokoro-onnx/releases/tag/model-files-v1.0>
//!
//! # Language Support
//!
//! | Voice prefix | Language | espeak-ng code | Notes |
//! |---|---|---|---|
//! | `af_`, `am_` | American English | `en-us` | Full support |
//! | `bf_`, `bm_` | British English | `en-gb` | Full support |
//! | `ef_`, `em_` | Spanish | `es` | Full support |
//! | `ff_` | French | `fr` | Full support |
//! | `hf_`, `hm_` | Hindi | `hi` | Full support |
//! | `if_`, `im_` | Italian | `it` | Full support |
//! | `jf_`, `jm_` | Japanese | `ja` | Functional via espeak-ng CJK |
//! | `pf_`, `pm_` | Brazilian Portuguese | `pt-br` | Full support |
//! | `zf_`, `zm_` | Mandarin Chinese | `cmn` | Functional via espeak-ng CJK |
//!
//! # Voice Naming Convention
//!
//! Voices follow the pattern `{language_prefix}_{name}`, e.g.:
//! - `af_heart` — American English female "heart"
//! - `bf_emma` — British English female "emma"
//! - `jf_alpha` — Japanese female "alpha"
//! - `zf_xiaobei` — Mandarin Chinese female "xiaobei"
//!
//! # Examples
//!
//! ## Basic Usage
//!
//! ```rust,no_run
//! use transcribe_rs::{SynthesisEngine, engines::kokoro::{KokoroEngine, KokoroInferenceParams}};
//! use std::path::PathBuf;
//!
//! let mut engine = KokoroEngine::new();
//! engine.load_model(&PathBuf::from("models/kokoro"))?;
//!
//! let result = engine.synthesize("Hello, world!", None)?;
//! println!("Generated {} samples at {}Hz", result.samples.len(), result.sample_rate);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ## With Custom Voice and Speed
//!
//! ```rust,no_run
//! use transcribe_rs::{SynthesisEngine, engines::kokoro::{KokoroEngine, KokoroInferenceParams}};
//! use std::path::PathBuf;
//!
//! let mut engine = KokoroEngine::new();
//! engine.load_model(&PathBuf::from("models/kokoro"))?;
//!
//! let params = KokoroInferenceParams {
//!     voice: "bf_emma".to_string(),
//!     speed: 0.9,
//!     ..Default::default()
//! };
//!
//! engine.synthesize_to_file("Hello from British Emma!", &PathBuf::from("out.wav"), Some(params))?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod engine;
pub mod model;
pub mod phonemizer;
pub mod vocab;
pub mod voices;

pub use engine::{KokoroEngine, KokoroInferenceParams, KokoroModelParams};
pub use model::KokoroError;
pub use phonemizer::EspeakConfig;
