use crate::actions::ACTION_MAP;
use log::{debug, error, warn};
use std::collections::HashSet;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_millis(30);

/// Commands processed sequentially by the coordinator thread.
enum Command {
    Input {
        binding_id: String,
        hotkey_string: String,
        is_pressed: bool,
    },
    ProcessingFinished,
}

/// Serialises shortcut lifecycle events through a single thread.
pub struct ActionCoordinator {
    tx: Sender<Command>,
}

pub fn is_speak_binding(id: &str) -> bool {
    id == "speak"
}

impl ActionCoordinator {
    pub fn new(app: tauri::AppHandle) -> Self {
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut last_press: Option<Instant> = None;
                let mut pressed_bindings: HashSet<String> = HashSet::new();

                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        Command::Input {
                            binding_id,
                            hotkey_string,
                            is_pressed,
                        } => {
                            if is_pressed {
                                let now = Instant::now();
                                let should_debounce = !is_speak_binding(&binding_id);
                                if should_debounce
                                    && last_press.is_some_and(|t| now.duration_since(t) < DEBOUNCE)
                                {
                                    debug!("Debounced press for '{binding_id}'");
                                    continue;
                                }
                                last_press = Some(now);

                                // Ignore repeat key-down events while key is held.
                                if !pressed_bindings.insert(binding_id.clone()) {
                                    debug!("Ignoring repeat key-down for '{binding_id}'");
                                    continue;
                                }
                            } else {
                                pressed_bindings.remove(&binding_id);
                            }

                            let Some(action) = ACTION_MAP.get(&binding_id) else {
                                warn!("No action in ACTION_MAP for '{binding_id}'");
                                continue;
                            };

                            if is_speak_binding(&binding_id) {
                                // Toggle mode: each key-down toggles start/stop.
                                if is_pressed {
                                    action.start(&app, &binding_id, &hotkey_string);
                                }
                            } else if is_pressed {
                                action.start(&app, &binding_id, &hotkey_string);
                            }
                        }
                        Command::ProcessingFinished => {
                            debug!("Coordinator received processing-finished event");
                        }
                    }
                }
                debug!("Shortcut coordinator exited");
            }));
            if let Err(e) = result {
                error!("Shortcut coordinator panicked: {e:?}");
            }
        });

        Self { tx }
    }

    /// Send a keyboard/signal input event for a speak binding.
    /// For signal-based toggles, send a press event followed by a release event.
    pub fn send_input(&self, binding_id: &str, hotkey_string: &str, is_pressed: bool) {
        if self
            .tx
            .send(Command::Input {
                binding_id: binding_id.to_string(),
                hotkey_string: hotkey_string.to_string(),
                is_pressed,
            })
            .is_err()
        {
            warn!("Shortcut coordinator channel closed");
        }
    }

    pub fn notify_processing_finished(&self) {
        if self.tx.send(Command::ProcessingFinished).is_err() {
            warn!("Shortcut coordinator channel closed");
        }
    }
}
