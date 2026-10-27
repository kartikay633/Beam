//! Explorer's right-click "Send to → Beam": a shortcut to this app in the
//! user's SendTo folder. Explorer starts the shortcut with the selected files
//! as arguments; the single-instance plugin hands them to the running app.

use std::path::PathBuf;

fn shortcut_path() -> Option<PathBuf> {
    // %APPDATA%\Microsoft\Windows\SendTo\Beam.lnk
    Some(
        dirs::data_dir()?
            .join("Microsoft")
            .join("Windows")
            .join("SendTo")
            .join("Beam.lnk"),
    )
}

/// Adds or removes the shortcut.
pub fn set_installed(install: bool) -> Result<(), String> {
    let path = shortcut_path().ok_or("could not find the SendTo folder")?;
    if !install {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    }
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        // COM gets its own thread, so its apartment can't clash with the
        // WebView's.
        std::thread::spawn(move || windows_link::create(&exe, &path))
            .join()
            .map_err(|_| "shortcut thread panicked".to_string())?
            .map_err(|e| format!("could not create the Send To shortcut: {e}"))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(windows)]
mod windows_link {
    use std::path::Path;

    use windows::core::{w, Interface, HSTRING};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    pub fn create(target: &Path, shortcut: &Path) -> windows::core::Result<()> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
            let result = (|| {
                let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
                let target = HSTRING::from(target);
                link.SetPath(&target)?;
                link.SetIconLocation(&target, 0)?;
                link.SetDescription(w!("Send with Beam"))?;
                link.cast::<IPersistFile>()?
                    .Save(&HSTRING::from(shortcut), true)
            })();
            CoUninitialize();
            result
        }
    }
}
