//! The tray icon: open the window, switch visibility, quit.

use beam_core::VisibilityMode;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::state::AppState;

/// The visibility items, kept so their check marks can follow changes made
/// elsewhere (the main window, or "everyone" expiring).
struct VisibilityItems {
    everyone: CheckMenuItem<Wry>,
    trusted: CheckMenuItem<Wry>,
    off: CheckMenuItem<Wry>,
}

pub fn create(app: &AppHandle, mode: VisibilityMode) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Beam", true, None::<&str>)?;
    let heading = MenuItem::with_id(app, "visibility", "Who can see me", false, None::<&str>)?;
    let everyone = CheckMenuItem::with_id(
        app,
        "vis-everyone",
        "Everyone (for 10 minutes)",
        true,
        mode == VisibilityMode::Everyone,
        None::<&str>,
    )?;
    let trusted = CheckMenuItem::with_id(
        app,
        "vis-trusted",
        "Trusted devices only",
        true,
        mode == VisibilityMode::TrustedOnly,
        None::<&str>,
    )?;
    let off = CheckMenuItem::with_id(
        app,
        "vis-off",
        "Off",
        true,
        mode == VisibilityMode::Off,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit Beam", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &PredefinedMenuItem::separator(app)?,
            &heading,
            &everyone,
            &trusted,
            &off,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Beam")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => crate::show_main(app),
            "vis-everyone" => set_visibility(app, VisibilityMode::Everyone),
            "vis-trusted" => set_visibility(app, VisibilityMode::TrustedOnly),
            "vis-off" => set_visibility(app, VisibilityMode::Off),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    app.manage(VisibilityItems {
        everyone,
        trusted,
        off,
    });
    Ok(())
}

fn set_visibility(app: &AppHandle, mode: VisibilityMode) {
    match app.try_state::<std::sync::Arc<AppState>>() {
        Some(state) => state.set_visibility(mode),
        None => show_visibility(app, mode),
    }
}

/// Moves the check mark to `mode`.
pub fn show_visibility(app: &AppHandle, mode: VisibilityMode) {
    if let Some(items) = app.try_state::<VisibilityItems>() {
        let _ = items.everyone.set_checked(mode == VisibilityMode::Everyone);
        let _ = items
            .trusted
            .set_checked(mode == VisibilityMode::TrustedOnly);
        let _ = items.off.set_checked(mode == VisibilityMode::Off);
    }
}
