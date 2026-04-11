# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Development Commands

**Prerequisites:** [Rust](https://rustup.rs/) (latest stable), [Bun](https://bun.sh/)

```bash
# Install dependencies
bun install

# Run in development mode
bun run tauri dev
# If cmake error on macOS:
CMAKE_POLICY_VERSION_MINIMUM=3.5 bun run tauri dev

# Build for production
bun run tauri build

# Linting and formatting (run before committing)
bun run lint              # ESLint for frontend
bun run lint:fix          # ESLint with auto-fix
bun run format            # Prettier + cargo fmt
bun run format:check      # Check formatting without changes
```

> **Note:** TTS models (Kokoro-82M) are downloaded at runtime by the user through the app UI — no bundled model setup is required for development.

## Architecture Overview

Parrot is a cross-platform desktop text-to-speech (TTS) app built with Tauri 2.x (Rust backend + React/TypeScript frontend).

### Backend Structure (`src-tauri/src/`)

**Core entry points:**

- `lib.rs` — Tauri setup, manager initialization, plugin wiring, command export via tauri-specta
- `main.rs` — Binary entry: CLI arg parsing, Linux GPU workaround, delegates to `lib.rs`
- `settings.rs` — `AppSettings` struct and persistence via `tauri-plugin-store` (`settings_store.json`)
- `cli.rs` — CLI argument definitions (clap derive macros)
- `signal_handle.rs` — Unix signal handlers (SIGUSR1/SIGUSR2) and shared `send_transcription_input()` helper

**Action pipeline:**

- `action_coordinator.rs` — Serializes shortcut lifecycle events through a single background thread; prevents concurrent TTS requests; debounces repeated keys (30ms)
- `actions.rs` — Action type definitions: `speak`, `cancel`, `play_pause`
- `audio_feedback.rs` — Plays start/stop audio cues via rodio
- `input.rs` — Enigo initialization for keyboard/mouse control (macOS requires accessibility permissions)

**Window management:**

- `overlay.rs` — Speaking overlay window lifecycle management
- `tray.rs` / `tray_i18n.rs` — System tray icon, context menu, and i18n-aware menu labels

**Utilities:**

- `shortcut.rs` — Global shortcut initialization and coordination
- `utils.rs` — Shared helpers (tray refresh, cancellation token management)

**Managers** (`managers/`):

- `tts.rs` — Core TTS engine: Kokoro model lifecycle, up to 2 parallel synthesis workers, crossfade between audio chunks (240 samples @ 24 kHz), audio playback via rodio/cpal, pause/resume, configurable model-unload timeout
- `model.rs` — Model catalog (Kokoro-82M), multi-component downloads with progress events, cancellation, extraction, and deletion
- `history.rs` — SQLite database (rusqlite) for TTS history: WAV file storage, configurable entry limit and retention policy

**Commands** (`commands/`):

- `mod.rs` — General: `get_app_settings`, `cancel_operation`, `toggle_tts_pause`, `preload_tts_model`, `get_model_status`, shortcut init/suspend/resume, permission helpers, `is_laptop`
- `audio.rs` — Audio devices: `get_available_output_devices`, `set_selected_output_device`, `play_test_sound`, `check_custom_sounds`
- `models.rs` — Models: `get_available_models`, `get_kokoro_voices`, `download_model`, `delete_model`, `set_active_model`, `cancel_download`
- `history.rs` — History: `get_history_entries`, `toggle_history_entry_saved`, `delete_history_entry`, `update_history_limit`, `update_history_retention_period`

**Audio Toolkit** (`audio_toolkit/`):

- `audio/device.rs` — Output device enumeration via cpal
- `audio/resampler.rs` — Audio resampling (rubato)
- `audio/utils.rs` — WAV file writing and audio utilities
- `constants.rs` — Shared audio constants

**Shortcut system** (`shortcut/`):

- Dual-backend architecture: **HandyKeys** (default on macOS) and **Tauri** via `tauri-plugin-global-shortcut` (default on Windows/Linux)
- Runtime switching via the `keyboard_implementation` setting; HandyKeys auto-falls back to Tauri with persistence on failure
- `handler.rs` — Routes shortcut events through ActionCoordinator

**Helpers** (`helpers/`):

- `clamshell.rs` — `is_laptop()`: detects laptop (clamshell) vs. desktop

### Frontend Structure (`src/`)

- `App.tsx` — Main shell: three-step onboarding (accessibility permissions → model selection → done), settings sidebar navigation, TTS error/no-selection toasts
- `bindings.ts` — **Manually maintained** Tauri command bindings; tauri-specta generates these only in debug builds — keep in sync by hand when adding/changing commands
- `stores/settingsStore.ts` — Zustand store for `AppSettings`; `settingUpdaters` map dispatches each settings key to the correct Rust command
- `stores/modelStore.ts` — Model download/selection state; listens to `download-progress` and `model-state-changed` backend events
- `hooks/useSettings.ts` — Thin wrapper around the settings store
- `components/settings/` — All settings UI, organized by feature area (40+ files)
- `components/model-selector/` — Model selection, download progress, and status UI
- `components/onboarding/` — First-run experience components
- `overlay/` — Separate Tauri window entry point for the speaking overlay shown during TTS playback

### Key Patterns

**Manager Pattern:** Three managers (TTSManager, ModelManager, HistoryManager) are initialized at startup and managed via Tauri's `.manage()` state.

**Command-Event Architecture:** Frontend → Backend via Tauri commands (request/response); Backend → Frontend via Tauri events (broadcast).

**TTS Flow:** Hotkey pressed → ActionCoordinator → TTSManager → parallel Kokoro synthesis → crossfade chunks → rodio playback → save to SQLite + WAV

**State Flow:** UI action → Zustand store → Tauri command → Rust state mutation → persistence (tauri-plugin-store JSON or SQLite)

**Shortcut Dual Backend:** HandyKeys (macOS) ↔ Tauri global shortcuts (Windows/Linux); switching persists to settings.

## Internationalization (i18n)

All user-facing strings must use i18next. ESLint enforces this — hardcoded strings in JSX will fail linting.

**Adding new text:**

1. Add the key to `src/i18n/locales/en/translation.json`
2. Use in component: `const { t } = useTranslation(); t('key.path')`

**Supported languages (17):**

| Code    | Language            | Notes           |
| ------- | ------------------- | --------------- |
| `en`    | English             | Source language |
| `zh`    | Simplified Chinese  |                 |
| `zh-TW` | Traditional Chinese |                 |
| `es`    | Spanish             |                 |
| `fr`    | French              |                 |
| `de`    | German              |                 |
| `ja`    | Japanese            |                 |
| `ko`    | Korean              |                 |
| `vi`    | Vietnamese          |                 |
| `pl`    | Polish              |                 |
| `it`    | Italian             |                 |
| `ru`    | Russian             |                 |
| `uk`    | Ukrainian           |                 |
| `pt`    | Portuguese          |                 |
| `cs`    | Czech               |                 |
| `tr`    | Turkish             |                 |
| `ar`    | Arabic              | RTL             |

**Adding a new language:**

1. Create `src/i18n/locales/{code}/translation.json`
2. Add metadata to `src/i18n/languages.ts`
3. For RTL languages, set `direction: "rtl"` in the metadata entry

**File structure:**

```
src/i18n/
├── index.ts              # i18n setup, language sync, RTL direction handling
├── languages.ts          # Language metadata (code, name, direction, priority)
└── locales/
    ├── en/translation.json   # English (source of truth)
    ├── ar/translation.json   # Arabic (RTL)
    └── ...                   # 15 other languages
```

## Code Style

**Rust:**

- Run `cargo fmt` and `cargo clippy` before committing
- Handle errors explicitly — avoid `unwrap()` in production code paths
- Add doc comments for public APIs

**TypeScript/React:**

- Strict TypeScript — avoid `any`
- Functional components with hooks
- Tailwind CSS for styling
- Path alias: `@/` → `./src/`
- When removing a feature, **delete** unused files entirely — do not stub them. TypeScript type-checks every file under `src/` due to `"include": ["src"]` in `tsconfig.json`

## Commit Guidelines

Use conventional commits:

- `feat:` — new features
- `fix:` — bug fixes
- `docs:` — documentation
- `refactor:` — code refactoring
- `chore:` — maintenance

## CLI Parameters

Parrot supports CLI flags for integration with scripts, window managers, and autostart configurations.

**Implementation files:**

- `src-tauri/src/cli.rs` — Argument definitions (clap derive)
- `src-tauri/src/main.rs` — Argument parsing before Tauri initialization
- `src-tauri/src/lib.rs` — CLI overrides applied in the setup closure and single-instance callback
- `src-tauri/src/signal_handle.rs` — Shared `send_transcription_input()` used by both signal handlers and CLI

**Available flags:**

| Flag                     | Description                                                                  |
| ------------------------ | ---------------------------------------------------------------------------- |
| `--toggle-transcription` | Trigger TTS speak on a running instance (via `tauri_plugin_single_instance`) |
| `--cancel`               | Cancel the current TTS operation on a running instance                       |
| `--start-hidden`         | Launch without showing the main window (tray icon still visible)             |
| `--no-tray`              | Launch without the system tray icon (closing window quits the app)           |
| `--debug`                | Enable debug mode with verbose (Trace) logging                               |

**Key design decisions:**

- CLI flags are runtime-only overrides — they do **not** modify persisted settings
- Remote flags (`--toggle-transcription`, `--cancel`) work by launching a second instance that forwards its args to the running instance via `tauri_plugin_single_instance`, then exits immediately
- `CliArgs` is stored in Tauri managed state (`.manage()`) so it's accessible in `on_window_event` and other handlers

## Debug Mode

Open debug panel: `Cmd+Shift+D` (macOS) / `Ctrl+Shift+D` (Windows/Linux)

## Platform Notes

- **macOS:** Accessibility permissions required for keyboard control; HandyKeys shortcut backend used by default; Metal rendering
- **Windows:** Tauri shortcut backend; code signing required for distribution
- **Linux:** Tauri shortcut backend; Wayland support is limited; speaking overlay disabled by default; requires GTK and gtk-layer-shell
