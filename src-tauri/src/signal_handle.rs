use crate::ActionCoordinator;
#[cfg(unix)]
use log::debug;
use log::warn;
use tauri::{AppHandle, Manager};

#[cfg(unix)]
use signal_hook::consts::{SIGUSR1, SIGUSR2};
#[cfg(unix)]
use signal_hook::iterator::Signals;
#[cfg(unix)]
use std::thread;

/// Send a shortcut input to the coordinator.
/// Used by signal handlers, CLI flags, and any other external trigger.
pub fn send_action_input(app: &AppHandle, binding_id: &str, source: &str) {
    if let Some(c) = app.try_state::<ActionCoordinator>() {
        c.send_input(binding_id, source, true);
        c.send_input(binding_id, source, false);
    } else {
        warn!("ActionCoordinator not initialized");
    }
}

#[cfg(unix)]
pub fn setup_signal_handler(app_handle: AppHandle, mut signals: Signals) {
    debug!("Signal handlers registered (SIGUSR1, SIGUSR2)");
    thread::spawn(move || {
        for sig in signals.forever() {
            let (binding_id, signal_name) = match sig {
                SIGUSR1 => ("speak", "SIGUSR1"),
                SIGUSR2 => ("speak", "SIGUSR2"),
                _ => continue,
            };
            debug!("Received {signal_name}");
            send_action_input(&app_handle, binding_id, signal_name);
        }
    });
}
