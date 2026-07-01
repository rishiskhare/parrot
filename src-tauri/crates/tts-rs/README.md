# tts-rs

A Rust library for text-to-speech synthesis using the [Kokoro](https://huggingface.co/hexgrad/Kokoro-82M) neural TTS model via ONNX inference.

## Features

- **Kokoro TTS engine** — natural-sounding neural speech via ONNX Runtime
- **Multiple voices** — 26 voices across 9 languages (English US & UK, Spanish, French, Hindi, Italian, Japanese, Portuguese Brazilian, Chinese Mandarin)
- **Streaming synthesis** — audio playback begins before the full text is synthesized
- **CPU-only** — no GPU required; runs efficiently on any modern CPU
- **Three precision levels** — f32, f16, and int8 model variants

## Installation

```toml
[dependencies]
tts-rs = { version = "2026.2.1", features = ["kokoro"] }
```

### Available Features

| Feature | Description | Dependencies |
|---------|-------------|--------------|
| `kokoro` | Kokoro neural TTS (ONNX) | `ort`, `ndarray`, `zip` |

No features are enabled by default. You must opt in explicitly.

## Model Files

Download the following files from the [taylorchu/kokoro-onnx v0.2.0 release](https://github.com/taylorchu/kokoro-onnx/releases/tag/v0.2.0):

| File | Size | Description |
|------|------|-------------|
| `kokoro-v1.0.onnx` | 310 MB | Full precision (f32) |
| `kokoro-v1.0.fp16.onnx` | 169 MB | Half precision (f16) |
| `kokoro-v1.0.int8.onnx` | 88 MB | Quantized (int8) — recommended |
| `voices-v1.0.bin` | — | Style vectors for all 26 voices (required) |

The `voices-v1.0.bin` file is required regardless of which model variant you use. Place all downloaded files in the same directory and pass that path to `load_model`.

## Usage

```rust
use tts_rs::engines::kokoro::KokoroEngine;
use std::path::PathBuf;

let mut engine = KokoroEngine::new();
engine.load_model(&PathBuf::from("models/kokoro"))?;

let audio = engine.synthesize("Hello, world!", Some("af_heart"), None)?;
// audio is a Vec<f32> of PCM samples at 24 kHz
```

## Running the Example

```sh
cargo run --example kokoro --features kokoro
```

## Acknowledgements

This library is derived from [transcribe-rs](https://github.com/cjpais/transcribe-rs) by
[CJ Pais](https://github.com/cjpais), which was itself built as the inference backend for the
[Handy](https://github.com/cjpais/handy) project. The original library supported multiple
speech-to-text (ASR) engines; this fork removes those entirely and repurposes the codebase
to focus exclusively on Kokoro TTS synthesis.

ONNX model files are provided by [taylorchu/kokoro-onnx](https://github.com/taylorchu/kokoro-onnx).
Additional reference and inspiration from [thewh1teagle/kokoro-onnx](https://github.com/thewh1teagle/kokoro-onnx).
The underlying TTS model is [Kokoro-82M](https://huggingface.co/hexgrad/Kokoro-82M) by [hexgrad](https://huggingface.co/hexgrad).

## License

[MIT](LICENSE)
