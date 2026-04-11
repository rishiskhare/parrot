use enigo::{Enigo, Key, Keyboard, Mouse, Settings};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

/// Wrapper for Enigo to store in Tauri's managed state.
/// Enigo is wrapped in a Mutex since it requires mutable access.
pub struct EnigoState(pub Mutex<Enigo>);

impl EnigoState {
    pub fn new() -> Result<Self, String> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|e| format!("Failed to initialize Enigo: {}", e))?;
        Ok(Self(Mutex::new(enigo)))
    }
}

/// Get the current mouse cursor position using the managed Enigo instance.
/// Returns None if the state is not available or if getting the location fails.
pub fn get_cursor_position(app_handle: &AppHandle) -> Option<(i32, i32)> {
    let enigo_state = app_handle.try_state::<EnigoState>()?;
    let enigo = enigo_state.0.lock().ok()?;
    enigo.location().ok()
}

/// Sends a copy command using platform-appropriate key combinations.
/// macOS: Cmd+C (Meta + virtual key code 8)
/// Windows/Linux: Ctrl+C
#[cfg(target_os = "macos")]
pub fn send_copy_ctrl_c(enigo: &mut Enigo) -> Result<(), String> {
    let (modifier_key, c_key_code) = (Key::Meta, Key::Other(8));

    enigo
        .key(modifier_key, enigo::Direction::Press)
        .map_err(|e| format!("Failed to press copy modifier key: {}", e))?;
    enigo
        .key(c_key_code, enigo::Direction::Click)
        .map_err(|e| format!("Failed to click C key: {}", e))?;

    std::thread::sleep(std::time::Duration::from_millis(50));

    enigo
        .key(modifier_key, enigo::Direction::Release)
        .map_err(|e| format!("Failed to release copy modifier key: {}", e))?;

    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn send_copy_ctrl_c(enigo: &mut Enigo) -> Result<(), String> {
    enigo
        .key(Key::Control, enigo::Direction::Press)
        .map_err(|e| format!("Failed to press Ctrl key: {}", e))?;
    enigo
        .key(Key::Unicode('c'), enigo::Direction::Click)
        .map_err(|e| format!("Failed to click C key: {}", e))?;

    std::thread::sleep(std::time::Duration::from_millis(50));

    enigo
        .key(Key::Control, enigo::Direction::Release)
        .map_err(|e| format!("Failed to release Ctrl key: {}", e))?;

    Ok(())
}
