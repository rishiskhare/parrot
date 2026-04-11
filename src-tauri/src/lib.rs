mod action_coordinator;
mod actions;
mod audio_feedback;
pub mod audio_toolkit;
pub mod cli;
mod commands;
mod helpers;
mod input;
mod managers;
mod overlay;
mod settings;
mod shortcut;
mod signal_handle;
mod text_normalization;
mod tray;
mod tray_i18n;
mod utils;

pub use cli::CliArgs;
use tauri_specta::{collect_commands, Builder};

pub use action_coordinator::ActionCoordinator;
use env_filter::Builder as EnvFilterBuilder;
use managers::history::HistoryManager;
use managers::model::ModelManager;
use managers::tts::TTSManager;
#[cfg(unix)]
use signal_hook::consts::{SIGUSR1, SIGUSR2};
#[cfg(unix)]
use signal_hook::iterator::Signals;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use tauri::image::Image;

use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Listener, Manager};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_log::{Builder as LogBuilder, RotationStrategy, Target, TargetKind};

use crate::settings::get_settings;

/// Resolve paths to the bundled espeak-ng binary and data directory.
///
/// Returns `(Option<PathBuf>, Option<PathBuf>)` — the binary path and data
/// directory.  These are passed to `KokoroModelParams` so tts-rs can locate
/// espeak-ng without relying on environment variables or PATH.
///
/// Best-effort: if the bundled files are missing (e.g. during `cargo test`
/// or a dev build without resources) we return `None` and tts-rs falls back
/// to system-installed `espeak-ng`.
fn resolve_bundled_espeak_ng(
    app_handle: &AppHandle,
) -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
    let resolver = app_handle.path();

    // --- espeak-ng binary ---------------------------------------------------
    #[cfg(not(target_os = "windows"))]
    let bin_name = "resources/espeak-ng/espeak-ng";
    #[cfg(target_os = "windows")]
    let bin_name = "resources/espeak-ng/espeak-ng.exe";

    let bin_path = resolver
        .resolve(bin_name, tauri::path::BaseDirectory::Resource)
        .ok()
        .filter(|p| p.exists())
        .inspect(|p| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = std::fs::metadata(p) {
                    let mut perms = meta.permissions();
                    perms.set_mode(perms.mode() | 0o111);
                    let _ = std::fs::set_permissions(p, perms);
                }
            }
            log::info!("Bundled espeak-ng binary: {}", p.display());
        });

    // --- espeak-ng-data directory --------------------------------------------
    let data_path = resolver
        .resolve(
            "resources/espeak-ng-data",
            tauri::path::BaseDirectory::Resource,
        )
        .ok()
        .filter(|p| p.is_dir())
        .inspect(|p| {
            log::info!("Bundled espeak-ng data: {}", p.display());
        });

    (bin_path, data_path)
}

// Global atomic to store the file log level filter
// We use u8 to store the log::LevelFilter as a number
pub static FILE_LOG_LEVEL: AtomicU8 = AtomicU8::new(log::LevelFilter::Debug as u8);

fn level_filter_from_u8(value: u8) -> log::LevelFilter {
    match value {
        0 => log::LevelFilter::Off,
        1 => log::LevelFilter::Error,
        2 => log::LevelFilter::Warn,
        3 => log::LevelFilter::Info,
        4 => log::LevelFilter::Debug,
        5 => log::LevelFilter::Trace,
        _ => log::LevelFilter::Trace,
    }
}

fn build_console_filter() -> env_filter::Filter {
    let mut builder = EnvFilterBuilder::new();

    match std::env::var("RUST_LOG") {
        Ok(spec) if !spec.trim().is_empty() => {
            if let Err(err) = builder.try_parse(&spec) {
                log::warn!(
                    "Ignoring invalid RUST_LOG value '{}': {}. Falling back to info-level console logging",
                    spec,
                    err
                );
                builder.filter_level(log::LevelFilter::Info);
            }
        }
        _ => {
            builder.filter_level(log::LevelFilter::Info);
        }
    }

    builder.build()
}

fn show_main_window(app: &AppHandle) {
    if let Some(main_window) = app.get_webview_window("main") {
        // First, ensure the window is visible
        if let Err(e) = main_window.show() {
            log::error!("Failed to show window: {}", e);
        }
        // Then, bring it to the front and give it focus
        if let Err(e) = main_window.set_focus() {
            log::error!("Failed to focus window: {}", e);
        }
        // Optional: On macOS, ensure the app becomes active if it was an accessory
        #[cfg(target_os = "macos")]
        {
            if let Err(e) = app.set_activation_policy(tauri::ActivationPolicy::Regular) {
                log::error!("Failed to set activation policy to Regular: {}", e);
            }
        }
    } else {
        log::error!("Main window not found.");
    }
}

fn initialize_core_logic(
    app_handle: &AppHandle,
    espeak_paths: (Option<std::path::PathBuf>, Option<std::path::PathBuf>),
) {
    // Note: Enigo (keyboard/mouse simulation) is NOT initialized here.
    // The frontend is responsible for calling the `initialize_enigo` command
    // after onboarding completes. This avoids triggering permission dialogs
    // on macOS before the user is ready.

    // Initialize the managers
    let model_manager =
        Arc::new(ModelManager::new(app_handle).expect("Failed to initialize model manager"));
    let history_manager =
        Arc::new(HistoryManager::new(app_handle).expect("Failed to initialize history manager"));
    let speech_manager = Arc::new(
        TTSManager::new(app_handle, model_manager.clone(), espeak_paths)
            .expect("Failed to initialize speech manager"),
    );

    // Add managers to Tauri's managed state
    app_handle.manage(model_manager.clone());
    app_handle.manage(history_manager.clone());
    app_handle.manage(speech_manager);

    // Note: Shortcuts are NOT initialized here.
    // The frontend is responsible for calling the `initialize_shortcuts` command
    // after permissions are confirmed (on macOS) or after onboarding completes.
    // This matches the pattern used for Enigo initialization.

    #[cfg(unix)]
    let signals = Signals::new([SIGUSR1, SIGUSR2]).unwrap();
    // Set up signal handlers for toggling transcription
    #[cfg(unix)]
    signal_handle::setup_signal_handler(app_handle.clone(), signals);

    // Apply macOS Accessory policy if starting hidden
    #[cfg(target_os = "macos")]
    {
        let settings = settings::get_settings(app_handle);
        if settings.start_hidden {
            let _ = app_handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
        }
    }
    // Get the current theme to set the appropriate initial icon
    let initial_theme = tray::get_current_theme(app_handle);

    // Choose the appropriate initial icon based on theme
    let initial_icon_path = tray::get_icon_path(initial_theme);

    let tray = TrayIconBuilder::new()
        .icon(
            Image::from_path(
                app_handle
                    .path()
                    .resolve(initial_icon_path, tauri::path::BaseDirectory::Resource)
                    .unwrap(),
            )
            .unwrap(),
        )
        .show_menu_on_left_click(true)
        .icon_as_template(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "settings" => {
                show_main_window(app);
            }
            "check_updates" => {
                let settings = settings::get_settings(app);
                if settings.update_checks_enabled {
                    show_main_window(app);
                    let _ = app.emit("check-for-updates", ());
                }
            }
            "copy_last_text" => {
                tray::copy_last_text(app);
            }
            "unload_model" => {
                let tts_manager = app.state::<Arc<TTSManager>>();
                if !tts_manager.is_model_loaded() {
                    log::warn!("No TTS model is currently loaded.");
                    return;
                }
                match tts_manager.unload_model() {
                    Ok(()) => log::info!("TTS model unloaded via tray."),
                    Err(e) => log::error!("Failed to unload TTS model via tray: {}", e),
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .build(app_handle)
        .unwrap();
    app_handle.manage(tray);

    // Initialize tray menu with idle state
    utils::update_tray_menu(app_handle, None);

    // Refresh tray menu when model state changes (e.g., to update "Unload Model" enabled state)
    let app_handle_for_listener = app_handle.clone();
    app_handle.listen("model-state-changed", move |_| {
        tray::update_tray_menu(&app_handle_for_listener, None);
    });

    // Apply show_tray_icon setting
    let settings = settings::get_settings(app_handle);
    if !settings.show_tray_icon {
        tray::set_tray_visibility(app_handle, false);
    }

    // Get the autostart manager and configure based on user setting
    let autostart_manager = app_handle.autolaunch();
    let settings = settings::get_settings(app_handle);

    if settings.autostart_enabled {
        // Enable autostart if user has opted in
        let _ = autostart_manager.enable();
    } else {
        // Disable autostart if user has opted out
        let _ = autostart_manager.disable();
    }

    // Create the speaking overlay window (hidden by default)
    utils::create_speaking_overlay(app_handle);
}

#[tauri::command]
#[specta::specta]
fn trigger_update_check(app: AppHandle) -> Result<(), String> {
    let settings = settings::get_settings(&app);
    if !settings.update_checks_enabled {
        return Ok(());
    }
    app.emit("check-for-updates", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run(cli_args: CliArgs) {
    // Parse console logging directives from RUST_LOG, falling back to info-level logging
    // when the variable is unset
    let console_filter = build_console_filter();

    let specta_builder = Builder::<tauri::Wry>::new().commands(collect_commands![
        shortcut::change_binding,
        shortcut::reset_binding,
        shortcut::change_audio_feedback_setting,
        shortcut::change_audio_feedback_volume_setting,
        shortcut::change_sound_theme_setting,
        shortcut::change_start_hidden_setting,
        shortcut::change_autostart_setting,
        shortcut::change_selected_language_setting,
        shortcut::change_kokoro_voice_setting,
        shortcut::change_overlay_position_setting,
        shortcut::change_debug_mode_setting,
        shortcut::change_show_close_button_setting,
        shortcut::change_experimental_enabled_setting,
        shortcut::suspend_binding,
        shortcut::resume_binding,
        shortcut::change_app_language_setting,
        shortcut::change_update_checks_setting,
        shortcut::change_keyboard_implementation_setting,
        shortcut::get_keyboard_implementation,
        shortcut::change_show_tray_icon_setting,
        shortcut::change_tts_workers_setting,
        shortcut::change_tts_speed_setting,
        shortcut::change_tts_shorten_first_chunk_setting,
        shortcut::handy_keys::start_handy_keys_recording,
        shortcut::handy_keys::stop_handy_keys_recording,
        trigger_update_check,
        commands::cancel_operation,
        commands::get_app_dir_path,
        commands::get_app_settings,
        commands::get_default_settings,
        commands::get_log_dir_path,
        commands::set_log_level,
        commands::open_history_folder,
        commands::open_log_dir,
        commands::open_app_data_dir,
        commands::initialize_enigo,
        commands::initialize_shortcuts,
        commands::models::get_available_models,
        commands::models::get_model_info,
        commands::models::get_kokoro_voices,
        commands::models::download_model,
        commands::models::delete_model,
        commands::models::cancel_download,
        commands::models::set_active_model,
        commands::models::get_current_model,
        commands::models::get_transcription_model_status,
        commands::models::is_model_loading,
        commands::models::has_any_models_available,
        commands::models::has_any_models_or_downloads,
        commands::get_model_status,
        commands::preload_tts_model,
        commands::toggle_tts_pause,
        commands::resize_overlay,
        commands::audio::get_available_output_devices,
        commands::audio::set_selected_output_device,
        commands::audio::get_selected_output_device,
        commands::audio::play_test_sound,
        commands::audio::check_custom_sounds,
        commands::history::get_history_entries,
        commands::history::toggle_history_entry_saved,
        commands::history::get_audio_file_path,
        commands::history::delete_history_entry,
        commands::history::update_history_limit,
        commands::history::update_history_retention_period,
        helpers::clamshell::is_laptop,
    ]);

    #[cfg(debug_assertions)] // <- Only export on non-release builds
    specta_builder
        .export(
            specta_typescript::Typescript::default()
                .bigint(specta_typescript::BigIntExportBehavior::Number),
            "../src/bindings.ts",
        )
        .expect("Failed to export typescript bindings");

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            LogBuilder::new()
                .level(log::LevelFilter::Trace) // Set to most verbose level globally
                .max_file_size(500_000)
                .rotation_strategy(RotationStrategy::KeepOne)
                .clear_targets()
                .targets([
                    // Console output respects RUST_LOG environment variable
                    Target::new(TargetKind::Stdout).filter({
                        let console_filter = console_filter.clone();
                        move |metadata| console_filter.enabled(metadata)
                    }),
                    // File logs respect the user's settings (stored in FILE_LOG_LEVEL atomic)
                    Target::new(TargetKind::LogDir {
                        file_name: Some("parrot".into()),
                    })
                    .filter(|metadata| {
                        let file_level = FILE_LOG_LEVEL.load(Ordering::Relaxed);
                        metadata.level() <= level_filter_from_u8(file_level)
                    }),
                ])
                .build(),
        );

    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());

    builder
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == "--toggle-transcription") {
                signal_handle::send_action_input(app, "speak", "CLI");
            } else if args.iter().any(|a| a == "--cancel") {
                crate::utils::cancel_current_operation(app);
            } else {
                show_main_window(app);
            }
        }))
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_macos_permissions::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .manage(cli_args.clone())
        .setup(move |app| {
            let mut settings = get_settings(app.handle());

            // CLI --debug flag overrides debug_mode and log level (runtime-only, not persisted)
            if cli_args.debug {
                settings.debug_mode = true;
                settings.log_level = settings::LogLevel::Trace;
            }

            let tauri_log_level: tauri_plugin_log::LogLevel = settings.log_level.into();
            let file_log_level: log::Level = tauri_log_level.into();
            // Store the file log level in the atomic for the filter to use
            FILE_LOG_LEVEL.store(file_log_level.to_level_filter() as u8, Ordering::Relaxed);
            let app_handle = app.handle().clone();
            app.manage(ActionCoordinator::new(app_handle.clone()));

            let espeak_paths = resolve_bundled_espeak_ng(&app_handle);
            initialize_core_logic(&app_handle, espeak_paths);

            // Hide tray icon if --no-tray was passed
            if cli_args.no_tray {
                tray::set_tray_visibility(&app_handle, false);
            }

            // Show main window only if not starting hidden
            // CLI --start-hidden flag overrides the setting
            let should_hide = settings.start_hidden || cli_args.start_hidden;
            if !should_hide {
                if let Some(main_window) = app_handle.get_webview_window("main") {
                    main_window.show().unwrap();
                    main_window.set_focus().unwrap();
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                let settings = get_settings(window.app_handle());
                let cli = window.app_handle().state::<CliArgs>();
                // If tray icon is hidden (via setting or --no-tray flag), quit the app
                if !settings.show_tray_icon || cli.no_tray {
                    window.app_handle().exit(0);
                    return;
                }
                api.prevent_close();
                let _res = window.hide();
                #[cfg(target_os = "macos")]
                {
                    let res = window
                        .app_handle()
                        .set_activation_policy(tauri::ActivationPolicy::Accessory);
                    if let Err(e) = res {
                        log::error!("Failed to set activation policy: {}", e);
                    }
                }
            }
            tauri::WindowEvent::ThemeChanged(theme) => {
                log::info!("Theme changed to: {:?}", theme);
                // Update tray icon to match new theme, maintaining idle state
                utils::change_tray_icon(window.app_handle());
            }
            _ => {}
        })
        .invoke_handler(specta_builder.invoke_handler())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
