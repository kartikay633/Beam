//! Stage 3 spike: `BeamWake.Task`, a WinRT background task that Windows
//! hosts (backgroundtaskhost.exe) when Beam's Bluetooth advert is heard. It
//! logs what it heard and starts the full-trust app, which is how a closed
//! Beam would be woken.

use std::io::Write;

use windows::core::{implement, IInspectable, Interface, Ref, Result, HRESULT, HSTRING};
use windows::ApplicationModel::Background::{
    IBackgroundTask, IBackgroundTaskInstance, IBackgroundTask_Impl,
};
use windows::ApplicationModel::FullTrustProcessLauncher;
use windows::Devices::Bluetooth::Background::BluetoothLEAdvertisementWatcherTriggerDetails;
use windows::Storage::ApplicationData;
use windows::Win32::System::WinRT::{IActivationFactory, IActivationFactory_Impl};

const CLASS_ID: &str = "BeamWake.Task";

fn log(line: &str) {
    // The task may run in an app container: write to the package's own folder.
    let Ok(dir) = ApplicationData::Current()
        .and_then(|d| d.LocalFolder())
        .and_then(|f| f.Path())
    else {
        return;
    };
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(format!("{dir}\\wake-task.log"))
    {
        let _ = writeln!(file, "{ms} pid {} {line}", std::process::id());
    }
}

#[implement(IBackgroundTask)]
struct WakeTask;

impl IBackgroundTask_Impl for WakeTask_Impl {
    fn Run(&self, instance: Ref<'_, IBackgroundTaskInstance>) -> Result<()> {
        let instance = instance.ok()?;
        let deferral = instance.GetDeferral()?;
        let heard = instance
            .TriggerDetails()
            .and_then(|d| d.cast::<BluetoothLEAdvertisementWatcherTriggerDetails>())
            .and_then(|d| d.Advertisements())
            .map(|adverts| {
                adverts
                    .into_iter()
                    .map(|a| {
                        format!(
                            "{:012x} {}dBm",
                            a.BluetoothAddress().unwrap_or(0),
                            a.RawSignalStrengthInDBm().unwrap_or(0)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|e| e.message().to_string());
        log(&format!("task ran: heard [{heard}]"));
        match FullTrustProcessLauncher::LaunchFullTrustProcessForCurrentAppAsync()
            .and_then(|op| op.join())
        {
            Ok(()) => log("launched the full-trust app"),
            Err(e) => log(&format!("launch failed: {:?} {}", e.code(), e.message())),
        }
        deferral.Complete()
    }
}

#[implement(IActivationFactory)]
struct Factory;

impl IActivationFactory_Impl for Factory_Impl {
    fn ActivateInstance(&self) -> Result<IInspectable> {
        Ok(WakeTask.into())
    }
}

/// How Windows asks a WinRT component for one of its classes.
///
/// # Safety
/// Called by the WinRT runtime with a valid class name and out pointer.
#[no_mangle]
pub unsafe extern "system" fn DllGetActivationFactory(
    class_id: std::mem::ManuallyDrop<HSTRING>,
    factory: *mut *mut core::ffi::c_void,
) -> HRESULT {
    if factory.is_null() {
        return windows::Win32::Foundation::E_POINTER;
    }
    if *class_id != HSTRING::from(CLASS_ID) {
        *factory = std::ptr::null_mut();
        return windows::Win32::Foundation::CLASS_E_CLASSNOTAVAILABLE;
    }
    let made: IActivationFactory = Factory.into();
    *factory = made.into_raw();
    HRESULT(0)
}

#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    // S_FALSE: keep it loaded; it's a spike.
    HRESULT(1)
}
