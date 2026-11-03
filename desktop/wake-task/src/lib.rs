//! `BeamWake.Task`: a background task Windows runs when a phone's Beam
//! Bluetooth advert appears while Beam isn't running. Beam registers it when
//! you quit from the tray and removes it when it starts (desktop
//! src-tauri/src/wake.rs); the task just starts Beam, quietly, in the tray.
//!
//! Only the Store (MSIX) package can use it: background tasks need package
//! identity. Windows hosts this DLL itself, because it won't give Bluetooth
//! triggers to a COM task in Beam's own exe (docs/STAGE3-SPIKE.md).

#![cfg(windows)]

use windows::core::{implement, IInspectable, Ref, Result, HRESULT, HSTRING};
use windows::ApplicationModel::Background::{
    IBackgroundTask, IBackgroundTaskInstance, IBackgroundTask_Impl,
};
use windows::ApplicationModel::FullTrustProcessLauncher;
use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, E_POINTER, S_OK};
use windows::Win32::System::WinRT::{IActivationFactory, IActivationFactory_Impl};

/// Matches the manifest (desktop/msix/AppxManifest.xml) and wake.rs.
const CLASS_ID: &str = "BeamWake.Task";
/// The manifest's parameter group for `--minimized`.
const PARAMETER_GROUP: &str = "Wake";

#[implement(IBackgroundTask)]
struct WakeTask;

impl IBackgroundTask_Impl for WakeTask_Impl {
    fn Run(&self, instance: Ref<'_, IBackgroundTaskInstance>) -> Result<()> {
        let deferral = instance.ok()?.GetDeferral()?;
        // If Beam is already running, its single-instance check hands this
        // launch to it, which does nothing for `--minimized`.
        let launched =
            FullTrustProcessLauncher::LaunchFullTrustProcessForCurrentAppWithParametersAsync(
                &HSTRING::from(PARAMETER_GROUP),
            )
            .and_then(|op| op.join());
        deferral.Complete()?;
        launched
    }
}

#[implement(IActivationFactory)]
struct Factory;

impl IActivationFactory_Impl for Factory_Impl {
    fn ActivateInstance(&self) -> Result<IInspectable> {
        Ok(WakeTask.into())
    }
}

/// How Windows asks this DLL for `BeamWake.Task`.
///
/// # Safety
/// Called by the Windows Runtime with a valid class name and out pointer.
#[no_mangle]
pub unsafe extern "system" fn DllGetActivationFactory(
    class_id: std::mem::ManuallyDrop<HSTRING>,
    factory: *mut *mut core::ffi::c_void,
) -> HRESULT {
    if factory.is_null() {
        return E_POINTER;
    }
    if class_id.to_string() != CLASS_ID {
        *factory = std::ptr::null_mut();
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let made: IActivationFactory = Factory.into();
    *factory = windows::core::Interface::into_raw(made);
    S_OK
}
