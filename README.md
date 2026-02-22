<div align="center">

  <img src="parrot.webp" alt="Parrot" width="120" />

  # Parrot: AI Text-to-Speech

  **Free, lightweight, local AI text-to-speech for your desktop**

  Highlight text in any app, press a shortcut<br/>
  Hear it read aloud *instantly, privately, on your device*

  Supports **9 languages:**<br/>
  English (US & UK) · Spanish · French · Hindi · Italian · Japanese · Portuguese (Brazilian) · Chinese (Mandarin)

  ![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey)
  ![License](https://img.shields.io/badge/license-MIT-green)
  ![Version](https://img.shields.io/badge/version-26.2.1-blue)
</div>

---

## What is Parrot?

Parrot reads your selected text aloud using a neural TTS model that runs entirely on your device. Your text never leaves your machine: no cloud, no accounts, no internet required after the initial model download.

The backend is written in Rust, keeping the app fast, lean, and resource-efficient. The model itself is only ~115 MB and runs on any modern CPU with no GPU required.

Select text. Press `Option+Space`. Done.

## Installation

### macOS (Apple Silicon)

**Homebrew:**
```sh
brew tap rishiskhare/parrot && brew install --cask parrot
```

**Manual:** Download [Parrot_26.2.1_aarch64.dmg](https://github.com/rishiskhare/parrot/releases/download/v26.2.1/Parrot_26.2.1_aarch64.dmg)

### Windows

| Architecture | Download |
|--------------|----------|
| x64 (Intel/AMD) | [Parrot_26.2.1_x64-setup.exe](https://github.com/rishiskhare/parrot/releases/download/v26.2.1/Parrot_26.2.1_x64-setup.exe) |
| ARM64 | [Parrot_26.2.1_arm64-setup.exe](https://github.com/rishiskhare/parrot/releases/download/v26.2.1/Parrot_26.2.1_arm64-setup.exe) |

### Linux

| Architecture | AppImage | Debian |
|--------------|----------|--------|
| x64 (Intel/AMD) | [Parrot_26.2.1_amd64.AppImage](https://github.com/rishiskhare/parrot/releases/download/v26.2.1/Parrot_26.2.1_amd64.AppImage) | [Parrot_26.2.1_amd64.deb](https://github.com/rishiskhare/parrot/releases/download/v26.2.1/Parrot_26.2.1_amd64.deb) |
| ARM64 | [Parrot_26.2.1_aarch64.AppImage](https://github.com/rishiskhare/parrot/releases/download/v26.2.1/Parrot_26.2.1_aarch64.AppImage) | [Parrot_26.2.1_arm64.deb](https://github.com/rishiskhare/parrot/releases/download/v26.2.1/Parrot_26.2.1_arm64.deb) |

> All downloads available on the [Releases](https://github.com/rishiskhare/parrot/releases) page.

On first launch, Parrot prompts you to download the TTS model (~115 MB). Once downloaded, the app works completely offline.

## Features

- **Private by design:** your text is processed locally and never sent anywhere
- **Lightweight:** ~115 MB model, minimal memory footprint, Rust-powered backend
- **Works in any app:** reads selected text from browsers, editors, PDFs, terminals, anywhere
- **Streaming playback:** audio starts playing before the full text has been synthesized
- **Free forever:** no subscription, no API key, no account required
- **Pause & resume:** pause and resume playback mid-sentence with a keyboard shortcut
- **Floating overlay:** a lightweight indicator shows speaking status with pause/cancel controls
- **History:** every utterance is saved with audio for replay or copy

## How It Works

1. **Select text** in any application
2. **Press the shortcut** (default: `Option+Space` on macOS, `Ctrl+Space` on Windows/Linux)
3. A small overlay appears while Parrot synthesizes and plays the audio
4. Press `Option+P` to pause/resume, or `Option+Escape` to cancel (all shortcuts are customizable)

## Models

Parrot ships with **Kokoro-82M**, a compact neural TTS model that delivers natural-sounding speech at ~115 MB, small enough to download once and forget, efficient enough to run on any modern CPU without a GPU.

Kokoro supports **54 voices** across 9 languages. The voice is selected automatically based on your language setting, or choose one manually in **Settings → General**.

| Language | Female Voices | Male Voices |
|----------|---------------|-------------|
| English (US) | Alloy, Aoede, Bella, Heart, Jessica, Kore, Nicole, Nova, River, Sarah, Sky | Adam, Echo, Eric, Fenrir, Liam, Michael, Onyx, Puck, Santa |
| English (UK) | Alice, Emma, Isabella, Lily | Daniel, Fable, George, Lewis |
| Spanish | Dora | Alex, Santa |
| French | Siwis | - |
| Hindi | Alpha, Beta | Omega, Psi |
| Italian | Sara | Nicola |
| Japanese | Alpha, Gongitsune, Nezumi, Tebukuro | Kumo |
| Portuguese (Brazilian) | Dora | Alex, Santa |
| Chinese (Mandarin) | Xiaobei, Xiaoni, Xiaoxiao, Xiaoyi | Yunjian, Yunxi, Yunxia, Yunyang |

## Keyboard Shortcuts

All shortcuts are fully customizable in **Settings → General**.

| Action | macOS | Windows / Linux |
|--------|-------|-----------------|
| Speak selected text | `Option+Space` | `Ctrl+Space` |
| Pause / resume playback | `Option+P` | `Alt+P` |
| Cancel / stop playback | `Option+Escape` | `Alt+Escape` |
| Open settings | `Cmd+,` | `Ctrl+,` |
| Open debug panel | `Cmd+Shift+D` | `Ctrl+Shift+D` |

The pause/resume and cancel shortcuts are only active while Parrot is playing. They can also be disabled entirely in **Settings → General** if you prefer not to capture those keys.

## Settings Overview

| Category | Options |
|----------|---------|
| **General** | Shortcuts, TTS language, voice, hold-to-speak, output device, audio feedback |
| **Models** | Download, switch, and delete TTS models |
| **Advanced → App** | Start hidden, autostart, tray icon, overlay position, model unload timeout |
| **Advanced → Speech** | Worker threads, playback speed, fast first response |
| **Advanced → History** | Entry limit, auto-delete period |
| **History** | Browse, replay, copy, and delete past utterances |
| **Debug** | Log level, keyboard implementation, diagnostics |

## Command-Line Interface

Parrot supports CLI flags for scripting and window manager integration. Remote control flags are delivered to the already-running instance; you do not need to keep a second instance running.

```
parrot [FLAGS]
```

| Flag | Description |
|------|-------------|
| `--toggle-transcription` | Toggle TTS on/off in the running instance |
| `--cancel` | Cancel the current playback |
| `--start-hidden` | Launch without showing the main window |
| `--no-tray` | Launch without a tray icon (closing the window quits) |
| `--debug` | Enable verbose trace logging |

**Example: bind to a window manager shortcut:**
```sh
parrot --toggle-transcription
```

> **macOS:** When using the app bundle, invoke the binary directly:
> ```sh
> /Applications/Parrot.app/Contents/MacOS/Parrot --toggle-transcription
> ```

## Linux Notes

### Text Input Tools

For reliable text pasting on Linux, install the appropriate tool for your display server:

| Display Server | Recommended | Install |
|----------------|-------------|---------|
| X11 | `xdotool` | `sudo apt install xdotool` |
| Wayland | `wtype` | `sudo apt install wtype` |
| Both | `dotool` | `sudo apt install dotool` |

`dotool` requires adding your user to the `input` group: `sudo usermod -aG input $USER` (log out and back in after).

### Global Shortcuts on Wayland

Parrot's built-in global shortcut capture has limited support on Wayland. The recommended approach is to configure your desktop environment or window manager to invoke the CLI flag instead.

**GNOME:**
1. Open **Settings > Keyboard > Keyboard Shortcuts > Custom Shortcuts**
2. Add a new shortcut with the command `parrot --toggle-transcription`

**KDE Plasma:**
1. Open **System Settings > Shortcuts > Custom Shortcuts**
2. Create a new **Command/URL** shortcut with `parrot --toggle-transcription`

**Sway / i3:**
```ini
bindsym $mod+o exec parrot --toggle-transcription
```

**Hyprland:**
```ini
bind = $mainMod, O, exec, parrot --toggle-transcription
```

### Unix Signal Control

You can also send signals directly to the Parrot process, useful for hotkey daemons that manage their own keybindings:

| Signal | Action |
|--------|--------|
| `SIGUSR1` | Toggle TTS |
| `SIGUSR2` | Toggle TTS |

```sh
pkill -USR2 -n parrot   # toggle TTS
```

### Other Linux Notes

- The speaking overlay is disabled by default on Linux (`Overlay Position: None`) because some compositors treat it as the active window and steal focus.
- If the app fails to start, try setting `WEBKIT_DISABLE_DMABUF_RENDERER=1`.
- If you see `error while loading shared libraries: libgtk-layer-shell.so.0`, install the runtime package:

  | Distro | Package | Command |
  |--------|---------|---------|
  | Ubuntu/Debian | `libgtk-layer-shell0` | `sudo apt install libgtk-layer-shell0` |
  | Fedora/RHEL | `gtk-layer-shell` | `sudo dnf install gtk-layer-shell` |
  | Arch | `gtk-layer-shell` | `sudo pacman -S gtk-layer-shell` |

## Building from Source

**Prerequisites:** [Rust](https://rustup.rs/) (latest stable), [Bun](https://bun.sh/)

```sh
# Clone the repository
git clone https://github.com/rishiskhare/parrot
cd parrot

# Install frontend dependencies
bun install

# Run in development mode
bun run tauri dev

# Build a release binary
bun run tauri build
```

> On macOS, if you hit a CMake error:
> ```sh
> CMAKE_POLICY_VERSION_MINIMUM=3.5 bun run tauri dev
> ```

## Architecture

Parrot is built with [Tauri 2](https://tauri.app/), a Rust backend with a React/TypeScript frontend. The entire synthesis and audio pipeline runs in Rust, which keeps CPU and memory usage low even during continuous playback.

```
src-tauri/src/
├── managers/
│   ├── tts.rs          # Streaming TTS synthesis and audio playback
│   ├── model.rs        # Model download, extraction, and lifecycle
│   └── history.rs      # Utterance storage and retention
├── audio_toolkit/      # Audio device enumeration and resampling
├── commands/           # Tauri IPC handlers (frontend ↔ backend)
├── settings.rs         # Persistent settings with serde
├── shortcut/           # Global hotkey capture (Tauri + HandyKeys backends)
└── overlay.rs          # Floating speaking indicator window

src/
├── components/settings/   # Settings UI (35+ components)
├── overlay/               # Speaking overlay window
└── stores/settingsStore.ts  # Zustand state management
```

**Key dependencies:** `tts-rs` (Kokoro TTS), `rodio` (audio playback), `cpal` (audio devices), `tauri-specta` (type-safe IPC)

## Acknowledgments

Parrot is a fork of [Handy](https://github.com/cjpais/Handy) by [CJ Pais](https://github.com/cjpais), released under the MIT License. The original project provided the Tauri architecture, audio pipeline, and UI foundation that made Parrot possible.

TTS synthesis is powered by [Kokoro-82M](https://huggingface.co/hexgrad/Kokoro-82M) via [tts-rs](https://github.com/rishiskhare/tts-rs).

## License

MIT. See [LICENSE](LICENSE) for full text.

Parrot is a derivative work of [Handy](https://github.com/cjpais/Handy) (© 2025 CJ Pais). Both are distributed under the MIT License.
