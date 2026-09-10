# Changelog

## [Unreleased]

### Fixed

- Persist and restore the model unload timeout setting after restart (GitHub issue #3)
- Hide Windows console windows spawned by espeak-ng during phonemization (GitHub issue #7)
- Keep HandyKeys shortcuts working when the main window is unfocused
- Animate overlay resize and keep overlay text in sync with playback after chunk crossfades

### Changed

- Synthesize with a single Kokoro engine instead of a parallel worker pool
- Keyboard implementation is a normal Advanced setting (no experimental gate)
- HandyKeys is the default shortcut backend on macOS and Windows; Linux stays on Tauri
- Linux window-manager bindings should send `SIGUSR2` to the running process (`pkill -USR2 -n parrot`) instead of a CLI flag
- Vendor `tts-rs` in-tree at `src-tauri/crates/tts-rs`

### Removed

- CLI flags (`--toggle-transcription`, `--cancel`, `--start-hidden`, `--no-tray`, `--debug`); use settings and Unix signals instead
- Worker-thread count setting

## [26.2.4] - 2026-02-25

See the [GitHub release](https://github.com/rishiskhare/parrot/releases/tag/v26.2.4) for binaries.

## [26.2.1] - 2026-02-22

### Added

- **Kokoro-82M TTS engine**: Neural text-to-speech running entirely on-device, no cloud required
- **54 voices across 9 languages**: English (US & UK), Spanish, French, Hindi, Italian, Japanese, Portuguese (Brazilian), Chinese (Mandarin)
- **Streaming playback**: Audio starts playing before the full text is synthesized
- **Floating overlay**: Lightweight speaking indicator with pause and cancel controls
- **Pause & resume**: Stop and continue playback mid-sentence via keyboard shortcut
- **History**: Every utterance saved with audio for replay or copy
- **Customizable shortcuts**: All keyboard shortcuts configurable in Settings
- **Audio feedback**: Optional start/stop sounds with volume control and theme selection
- **Output device selection**: Route audio to any connected output device
- **Model unload timeout**: Automatically free memory after a period of inactivity
- **Auto-updater**: In-app update notifications and one-click installation
- **Unix signal control**: `SIGUSR1`/`SIGUSR2` for hotkey daemon integration on Linux
- Cross-platform: macOS, Windows, Linux (x86 and ARM)
