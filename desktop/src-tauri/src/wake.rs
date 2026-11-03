//! Waking Beam after it was quit (docs/STAGE3-SPIKE.md). When you quit from
//! the tray, Beam asks Windows to run `BeamWake.Task` (desktop/wake-task)
//! whenever a phone's Beam Bluetooth advert appears; the task starts Beam
//! again, quietly, in the tray. Beam removes the request when it starts, so a
//! running Beam isn't relaunched every few seconds.
//!
//! Background tasks need package identity: this only works in the Store
//! (MSIX) package, and is skipped by the MSI install.

/// The registration's name, so Beam can find and remove its own.
const TASK_NAME: &str = "BeamWake";
/// The background task's class, as declared in desktop/msix/AppxManifest.xml.
const ENTRY_POINT: &str = "BeamWake.Task";
/// Startup task id in the manifest (Settings → Start with Windows).
const STARTUP_TASK_ID: &str = "BeamStartup";

#[cfg(windows)]
mod win {
    use beam_core::ble;
    use windows::core::HSTRING;
    use windows::ApplicationModel::Activation::ActivationKind;
    use windows::ApplicationModel::AppInstance;
    use windows::ApplicationModel::Background::{
        BackgroundExecutionManager, BackgroundTaskBuilder, BackgroundTaskRegistration,
        BluetoothLEAdvertisementWatcherTrigger,
    };
    use windows::ApplicationModel::{Package, StartupTask, StartupTaskState};
    use windows::Devices::Bluetooth::Advertisement::BluetoothLEAdvertisementBytePattern;
    use windows::Storage::Streams::DataWriter;

    use super::{ENTRY_POINT, STARTUP_TASK_ID, TASK_NAME};

    /// True in the Store package (it has package identity).
    pub fn is_packaged() -> bool {
        Package::Current().is_ok()
    }

    /// True when Windows started the package at sign-in (its startup task),
    /// which, like `--minimized`, should leave Beam in the tray.
    pub fn started_at_sign_in() -> bool {
        AppInstance::GetActivatedEventArgs()
            .and_then(|args| args.Kind())
            .is_ok_and(|kind| kind == ActivationKind::StartupTask)
    }

    pub fn disarm() -> windows::core::Result<()> {
        for pair in BackgroundTaskRegistration::AllTasks()? {
            let task = pair.Value()?;
            if task.Name()? == TASK_NAME {
                task.Unregister(true)?;
            }
        }
        Ok(())
    }

    pub fn arm() -> windows::core::Result<()> {
        disarm()?;
        BackgroundExecutionManager::RequestAccessAsync()?.join()?;
        let trigger = BluetoothLEAdvertisementWatcherTrigger::new()?;
        // One pattern (Windows allows one in the background): the 128-bit
        // service-data section (0x21) starting with Beam's service UUID.
        let mut uuid = *uuid::Uuid::parse_str(ble::SERVICE_UUID)
            .expect("valid")
            .as_bytes();
        uuid.reverse();
        let writer = DataWriter::new()?;
        writer.WriteBytes(&uuid)?;
        let pattern =
            BluetoothLEAdvertisementBytePattern::Create(0x21, 0, &writer.DetachBuffer()?)?;
        trigger
            .AdvertisementFilter()?
            .BytePatterns()?
            .Append(&pattern)?;
        let builder = BackgroundTaskBuilder::new()?;
        builder.SetTrigger(&trigger)?;
        builder.SetName(&HSTRING::from(TASK_NAME))?;
        builder.SetTaskEntryPoint(&HSTRING::from(ENTRY_POINT))?;
        builder.Register()?;
        Ok(())
    }

    /// Start with Windows, for the package (it can't use a Run key).
    pub fn set_startup(enabled: bool) -> windows::core::Result<bool> {
        let task = StartupTask::GetAsync(&HSTRING::from(STARTUP_TASK_ID))?.join()?;
        if !enabled {
            task.Disable()?;
            return Ok(true);
        }
        let state = task.RequestEnableAsync()?.join()?;
        Ok(matches!(
            state,
            StartupTaskState::Enabled | StartupTaskState::EnabledByPolicy
        ))
    }
}

#[cfg(windows)]
pub use win::{is_packaged, started_at_sign_in};

#[cfg(not(windows))]
pub fn is_packaged() -> bool {
    false
}

#[cfg(not(windows))]
pub fn started_at_sign_in() -> bool {
    false
}

/// Runs a blocking WinRT call on its own (multithreaded) thread, so it can't
/// stall or deadlock the UI thread it's called from.
#[cfg(windows)]
fn off_ui_thread<T: Send + 'static>(
    call: impl FnOnce() -> windows::core::Result<T> + Send + 'static,
) -> windows::core::Result<T> {
    std::thread::spawn(call).join().unwrap_or_else(|_| {
        Err(windows::core::Error::from(
            windows::Win32::Foundation::E_UNEXPECTED,
        ))
    })
}

/// Called when quitting from the tray: wake Beam when a phone shows up.
pub fn arm() {
    #[cfg(windows)]
    if is_packaged() {
        if let Err(e) = off_ui_thread(win::arm) {
            crate::offline::log(format!(
                "could not set up waking on Bluetooth: {}",
                e.message()
            ));
        }
    }
}

/// Called when Beam starts: it's running, so nothing needs to wake it.
pub fn disarm() {
    #[cfg(windows)]
    if is_packaged() {
        if let Err(e) = off_ui_thread(win::disarm) {
            crate::offline::log(format!(
                "could not remove the Bluetooth wake-up: {}",
                e.message()
            ));
        }
    }
}

/// Start with Windows in the package, through its startup task. `None`
/// outside the package (the caller uses the autostart plugin there).
pub fn set_startup(enabled: bool) -> Option<Result<(), String>> {
    #[cfg(windows)]
    if is_packaged() {
        return Some(match off_ui_thread(move || win::set_startup(enabled)) {
            Ok(true) => Ok(()),
            Ok(false) => Err(
                "Windows didn't allow Beam to start with Windows. Turn it on in Settings → Apps → Startup."
                    .to_string(),
            ),
            Err(e) => Err(format!("could not change start-with-Windows: {}", e.message())),
        });
    }
    let _ = (enabled, STARTUP_TASK_ID);
    None
}
