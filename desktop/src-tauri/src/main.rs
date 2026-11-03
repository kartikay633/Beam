//! Beam for Windows: a tray app that sends and receives files with beam-core.

// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod keystore;
mod offline;
mod pairing;
mod sendto;
mod settings;
mod state;
mod tray;

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Manager, RunEvent, WebviewWindowBuilder};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

use crate::state::AppState;

/// Passed when Windows starts Beam at login: stay in the tray.
const MINIMIZED_FLAG: &str = "--minimized";

fn main() {
    tauri::Builder::default()
        // Must be first: a second launch (for example Explorer's Send To)
        // hands its arguments to the running app and exits.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            stage_files_from_args(app, &argv);
            show_main(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![MINIMIZED_FLAG]),
        ))
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let state = AppState::new(app.handle().clone(), config_dir)?;
            app.manage(state.clone());
            tray::create(app.handle(), state.visibility.mode())?;
            if state.settings().send_to_shortcut {
                // Keeps the shortcut pointing at this copy of the app.
                if let Err(e) = sendto::set_installed(true) {
                    eprintln!("Send To shortcut: {e}");
                }
            }

            let args: Vec<String> = std::env::args().collect();
            stage_files_from_args(app.handle(), &args);
            if !args.iter().any(|a| a == MINIMIZED_FLAG) {
                show_main(app.handle());
            }
            tauri::async_runtime::spawn(async move {
                if let Err(e) = state.start().await {
                    eprintln!("Beam could not start receiving: {e}");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::get_offer,
            commands::respond_offer,
            commands::stage_files,
            commands::unstage_files,
            commands::send_staged,
            commands::cancel_transfer,
            commands::clear_finished,
            commands::dismiss_notice,
            commands::set_visibility,
            commands::set_auto_accept,
            commands::remove_trusted,
            commands::update_settings,
            commands::pairing_info,
            commands::open_folder,
        ])
        .build(tauri::generate_context!())
        .expect("Beam failed to start")
        .run(|_app, event| {
            // Closing the last window keeps Beam running in the tray; only
            // Quit (an explicit exit code) ends it.
            if let RunEvent::ExitRequested {
                code: None, api, ..
            } = event
            {
                api.prevent_exit();
            }
        });
}

/// Shows the main window, creating it if needed. Closing it destroys it
/// (and its WebView2 processes), so Beam waiting in the tray stays small.
pub fn show_main(app: &AppHandle) {
    let window = match app.get_webview_window("main") {
        Some(window) => window,
        None => {
            let Some(config) = app.config().app.windows.iter().find(|w| w.label == "main") else {
                return;
            };
            match WebviewWindowBuilder::from_config(app, config).and_then(|b| b.build()) {
                Ok(window) => window,
                Err(e) => {
                    eprintln!("could not open the Beam window: {e}");
                    return;
                }
            }
        }
    };
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

/// Files passed on the command line (by Send To) are staged for sending.
fn stage_files_from_args(app: &AppHandle, argv: &[String]) {
    let paths: Vec<PathBuf> = argv
        .iter()
        .skip(1)
        .filter(|arg| !arg.starts_with("--"))
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .collect();
    if paths.is_empty() {
        return;
    }
    if let Some(state) = app.try_state::<Arc<AppState>>() {
        state.stage(paths);
    }
}

pub fn set_autostart(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let launcher = app.autolaunch();
    let result = if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    };
    result.map_err(|e| format!("could not change start-with-Windows: {e}"))
}
