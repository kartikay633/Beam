//! Commands the UI calls with `invoke`. Each is a thin wrapper around
//! [`AppState`]; errors are returned as strings for the UI to show.

use std::path::PathBuf;
use std::sync::Arc;

use beam_core::VisibilityMode;
use tauri::State;
use tauri_plugin_opener::OpenerExt;

use crate::pairing::{pairing_info as build_pairing_info, PairingInfo};
use crate::settings::Settings;
use crate::state::{AppState, OfferView, Snapshot};

type AppStateRef<'a> = State<'a, Arc<AppState>>;

#[tauri::command]
pub fn get_snapshot(state: AppStateRef<'_>) -> Snapshot {
    state.snapshot()
}

#[tauri::command]
pub fn get_offer(state: AppStateRef<'_>, id: String) -> Option<OfferView> {
    state.offer(&id)
}

#[tauri::command]
pub fn respond_offer(
    state: AppStateRef<'_>,
    id: String,
    accept: bool,
    trust: bool,
) -> Result<(), String> {
    state.respond(&id, accept, trust)
}

#[tauri::command]
pub fn stage_files(state: AppStateRef<'_>, paths: Vec<String>) {
    state.stage(paths.into_iter().map(PathBuf::from).collect());
}

/// Removes one staged file, or all of them when `path` is missing.
#[tauri::command]
pub fn unstage_files(state: AppStateRef<'_>, path: Option<String>) {
    state.unstage(path.map(PathBuf::from));
}

/// Sends the staged files to a device id seen on the network, or an address.
#[tauri::command]
pub fn send_staged(state: AppStateRef<'_>, target: String) -> Result<(), String> {
    state.send_staged(target)
}

#[tauri::command]
pub fn cancel_transfer(state: AppStateRef<'_>, id: String) {
    state.cancel_transfer(&id);
}

#[tauri::command]
pub fn clear_finished(state: AppStateRef<'_>) {
    state.clear_finished();
}

#[tauri::command]
pub fn set_visibility(state: AppStateRef<'_>, mode: String) -> Result<(), String> {
    let mode: VisibilityMode = mode.parse()?;
    state.set_visibility(mode);
    Ok(())
}

#[tauri::command]
pub fn set_auto_accept(
    state: AppStateRef<'_>,
    device_id: String,
    auto_accept: bool,
) -> Result<(), String> {
    state.set_auto_accept(&device_id, auto_accept)
}

#[tauri::command]
pub fn remove_trusted(state: AppStateRef<'_>, device_id: String) -> Result<(), String> {
    state.remove_trusted(&device_id)
}

#[tauri::command]
pub async fn update_settings(state: AppStateRef<'_>, settings: Settings) -> Result<(), String> {
    state.update_settings(settings).await
}

#[tauri::command]
pub fn pairing_info(state: AppStateRef<'_>) -> PairingInfo {
    build_pairing_info(&state.device, state.port(), &state.pairing)
}

/// Opens a folder in Explorer (the download folder by default).
#[tauri::command]
pub fn open_folder(state: AppStateRef<'_>, path: Option<String>) -> Result<(), String> {
    let folder = path
        .map(PathBuf::from)
        .unwrap_or_else(|| state.download_dir());
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    state
        .app
        .opener()
        .open_path(folder.display().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}
