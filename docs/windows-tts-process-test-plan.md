# Windows TTS Process Execution Test Plan

## Root cause covered by this change

Parrot's Kokoro TTS path uses `tts-rs`, which launches `espeak-ng.exe` as a
child process for phonemization. On Windows, `espeak-ng.exe` is a console
subsystem binary. If it is spawned from the background Tauri app without
`CREATE_NO_WINDOW`, Windows may create a visible console window for each child
process invocation.

## What this patch changes

- Parrot now pins `tts-rs` via a local `[patch.crates-io]` override.
- The vendored `tts-rs` phonemizer sets `CREATE_NO_WINDOW` for Windows
  `espeak-ng` child processes.
- macOS and Linux behavior is unchanged.

## Manual verification on a real Windows machine

1. Build and install Parrot with this patch on Windows.
2. Launch Parrot normally from the Start menu.
3. Trigger TTS once on a short selection.
   Expected: speech is generated and no `cmd.exe` or console window appears.
4. Trigger TTS repeatedly 20-30 times in a row.
   Expected: no visible console windows appear over time and focus does not
   leave the active app.
5. Trigger TTS on a long selection that is chunked into multiple synthesis
   requests.
   Expected: no console windows appear while chunks are processed.
6. Leave Parrot running for at least 15 minutes, then trigger TTS again.
   Expected: first request after idle still produces no visible console window.
7. Hide Parrot to the tray and trigger TTS from the tray-driven workflow.
   Expected: behavior matches a normal launch, with no visible console windows.
8. While TTS is active, keep typing in another app.
   Expected: no focus stealing and no interruption from background processes.

## Optional observability checks

- Use Process Explorer or Process Monitor to confirm `espeak-ng.exe` is created
  as a background child of Parrot without a visible console window.
- If any window still appears, capture the exact process name so the remaining
  spawn path can be isolated.
