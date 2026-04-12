//! Speech synthesis engines.
//!
//! This module contains implementations of text-to-speech engines.
//!
//! # Available Engines
//!
//! Enable engines via Cargo features:
//! - `kokoro` - Kokoro TTS (ONNX format, espeak-ng required)

#[cfg(feature = "kokoro")]
pub mod kokoro;
