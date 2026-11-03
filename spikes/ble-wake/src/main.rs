//! Stage 3 spike (docs/PLAN-SEND-TO-CLOSED-PC.md): can Windows start a
//! packaged Beam when a phone's Bluetooth advert appears, with Beam closed?
//!
//! Run normally (from its package) it registers a background task with a
//! `BluetoothLEAdvertisementWatcherTrigger` that matches Beam's service data.
//! Windows then starts it as a COM server (`-RegisterProcessAsComServer`)
//! whenever a matching advert is heard, and the task logs what it heard.
//! Everything goes to `LOG`, so the run can be read afterwards.

#![windows_subsystem = "windows"]

use std::io::Write;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows::core::{implement, Interface, Ref, Result, BOOL, GUID, HSTRING};
use windows::ApplicationModel::Background::{
    BackgroundExecutionManager, BackgroundTaskBuilder, BackgroundTaskRegistration,
    BluetoothLEAdvertisementWatcherTrigger, IBackgroundTask, IBackgroundTaskInstance,
    IBackgroundTask_Impl,
};
use windows::Devices::Bluetooth::Advertisement::BluetoothLEAdvertisementBytePattern;
use windows::Devices::Bluetooth::Background::BluetoothLEAdvertisementWatcherTriggerDetails;
use windows::Storage::Streams::DataWriter;
use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_POINTER};
use windows::Win32::System::Com::{
    CoInitializeEx, CoRegisterClassObject, CoRevokeClassObject, IClassFactory, IClassFactory_Impl,
    CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, REGCLS_MULTIPLEUSE,
};
use windows_core::IUnknown;

/// Must match the `com:Class Id` in AppxManifest.xml.
const CLSID_WAKE: GUID = GUID::from_u128(0x6f1d2a4e_8c3b_4e7a_9d55_2b7e1c9a4f10);
const TASK_NAME: &str = "BeamWakeSpike";
/// Outside the package's redirected folders, so it's easy to read.
const LOG: &str = r"D:\beam-target\wake-spike.log";
/// Beam's service UUID (core/src/ble.rs), least significant byte first, as
/// it appears in a 128-bit service-data section (type 0x21).
const SERVICE_UUID_LE: [u8; 16] = [
    0x2b, 0x5e, 0xb2, 0x6a, 0x4e, 0xc7, 0xb0, 0x96, 0x17, 0x47, 0xbd, 0xdd, 0x3f, 0x89, 0x59, 0xaf,
];

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn log(line: impl AsRef<str>) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG)
    {
        let _ = writeln!(file, "{} pid {} {}", now_ms(), std::process::id(), line.as_ref());
    }
}

#[implement(IBackgroundTask)]
struct WakeTask;

impl IBackgroundTask_Impl for WakeTask_Impl {
    fn Run(&self, instance: Ref<'_, IBackgroundTaskInstance>) -> Result<()> {
        let instance = instance.ok()?;
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
            });
        log(format!("task ran: heard [{}]", heard.unwrap_or_else(|e| e.message().to_string())));
        Ok(())
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<'_, IUnknown>,
        iid: *const GUID,
        object: *mut *mut core::ffi::c_void,
    ) -> Result<()> {
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        if iid.is_null() || object.is_null() {
            return Err(E_POINTER.into());
        }
        let task: IBackgroundTask = WakeTask.into();
        unsafe { task.query(iid, object).ok() }
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

/// Started by Windows for the task: serve the class, then exit after a while.
fn serve() -> Result<()> {
    log("started as COM server");
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
    let factory: IClassFactory = Factory.into();
    let token = unsafe {
        CoRegisterClassObject(&CLSID_WAKE, &factory, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE)?
    };
    std::thread::sleep(Duration::from_secs(20));
    unsafe { CoRevokeClassObject(token)? };
    log("COM server exiting");
    Ok(())
}

/// Launched normally: (re)register the Bluetooth-triggered task.
fn register() -> Result<()> {
    let access = BackgroundExecutionManager::RequestAccessAsync()?.join();
    log(format!("background access: {access:?}"));


    // Which variant to try, from a file (packaged apps launch without args).
    let mode = std::fs::read_to_string(r"D:\beam-target\wake-spike-mode.txt")
        .unwrap_or_default()
        .trim()
        .to_string();
    log(format!("mode {mode:?}"));
    if let Ok(adapter) = windows::Devices::Bluetooth::BluetoothAdapter::GetDefaultAsync()?.join() {
        log(format!(
            "adapter: LE {:?} central {:?} peripheral {:?} extended adv {:?} offload {:?}",
            adapter.IsLowEnergySupported(),
            adapter.IsCentralRoleSupported(),
            adapter.IsPeripheralRoleSupported(),
            adapter.IsExtendedAdvertisingSupported(),
            adapter.IsAdvertisementOffloadSupported(),
        ));
    }
    if mode == "status" {
        for pair in BackgroundTaskRegistration::AllTasks()? {
            let task = pair.Value()?;
            log(format!("registered: {} {:?}", task.Name()?, pair.Key()?));
        }
        log("status done");
        return Ok(());
    }
    if mode == "launched" {
        log("woken: started by the background task");
        return Ok(());
    }
    for pair in BackgroundTaskRegistration::AllTasks()? {
        let task = pair.Value()?;
        if task.Name()? == HSTRING::from(TASK_NAME) {
            task.Unregister(true)?;
            log("removed the previous registration");
        }
    }
    if mode == "time" {
        let builder = BackgroundTaskBuilder::new()?;
        builder.SetTrigger(&windows::ApplicationModel::Background::TimeTrigger::Create(15, false)?)?;
        builder.SetName(&HSTRING::from(TASK_NAME))?;
        builder.SetTaskEntryPointClsid(CLSID_WAKE)?;
        match builder.Register() {
            Ok(r) => log(format!("registered time task {:?}", r.TaskId())),
            Err(e) => log(format!("time register failed: {:?} {}", e.code(), e.message())),
        }
        return Ok(());
    }
    let trigger = BluetoothLEAdvertisementWatcherTrigger::new()?;
    if mode == "rssi" {
        let filter = trigger.SignalStrengthFilter()?;
        filter.SetInRangeThresholdInDBm(&windows::Foundation::PropertyValue::CreateInt16(-90)?.cast::<windows::Foundation::IReference<i16>>()?)?;
        filter.SetOutOfRangeThresholdInDBm(&windows::Foundation::PropertyValue::CreateInt16(-100)?.cast::<windows::Foundation::IReference<i16>>()?)?;
        filter.SetOutOfRangeTimeout(&windows::Foundation::PropertyValue::CreateTimeSpan(windows::Foundation::TimeSpan { Duration: 20_000_000 })?.cast::<windows::Foundation::IReference<windows::Foundation::TimeSpan>>()?)?;
        filter.SetSamplingInterval(&windows::Foundation::PropertyValue::CreateTimeSpan(windows::Foundation::TimeSpan { Duration: 10_000_000 })?.cast::<windows::Foundation::IReference<windows::Foundation::TimeSpan>>()?)?;
        trigger.SetSignalStrengthFilter(&filter)?;
    }
    if mode == "uuid" {
        let uuid = GUID::from_u128(0xaf59893f_ddbd_4717_96b0_c74e6ab25e2b);
        trigger.AdvertisementFilter()?.Advertisement()?.ServiceUuids()?.Append(uuid)?;
        return finish(&trigger);
    }
    let writer = DataWriter::new()?;
    let bytes: &[u8] = if mode == "short" { &SERVICE_UUID_LE[..4] } else { &SERVICE_UUID_LE };
    writer.WriteBytes(bytes)?;
    let pattern = BluetoothLEAdvertisementBytePattern::Create(
        0x21,
        0,
        &writer.DetachBuffer()?,
    )?;
    trigger.AdvertisementFilter()?.BytePatterns()?.Append(&pattern)?;
    log(format!(
        "trigger: sampling {:?}..{:?}, out-of-range {:?}..{:?}",
        trigger.MinSamplingInterval().map(|d| d.Duration),
        trigger.MaxSamplingInterval().map(|d| d.Duration),
        trigger.MinOutOfRangeTimeout().map(|d| d.Duration),
        trigger.MaxOutOfRangeTimeout().map(|d| d.Duration),
    ));

    finish(&trigger)
}

fn finish(trigger: &BluetoothLEAdvertisementWatcherTrigger) -> Result<()> {
    let builder = BackgroundTaskBuilder::new()?;
    builder.SetTrigger(trigger)?;
    builder.SetName(&HSTRING::from(TASK_NAME))?;
    let winrt = std::fs::read_to_string(r"D:\beam-target\wake-spike-mode.txt")
        .is_ok_and(|m| m.trim() == "winrt");
    if winrt {
        // Hosted by Windows: the WinRT class in beam_wake_task.dll.
        builder.SetTaskEntryPoint(&HSTRING::from("BeamWake.Task"))?;
    } else {
        builder.SetTaskEntryPointClsid(CLSID_WAKE)?;
    }
    match builder.Register() {
        Ok(registration) => log(format!("registered task {:?} (winrt {winrt})", registration.TaskId())),
        Err(e) => log(format!("register failed: {:?} {}", e.code(), e.message())),
    }
    Ok(())
}

fn main() {
    let as_server = std::env::args().any(|a| a.eq_ignore_ascii_case("-RegisterProcessAsComServer"));
    let result = if as_server { serve() } else { register() };
    if let Err(e) = result {
        log(format!("error: {:?} {}", e.code(), e.message()));
    }
}
