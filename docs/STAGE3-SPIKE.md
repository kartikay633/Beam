# Stage 3 spike: waking a closed Beam over Bluetooth

**Built into Beam 0.2.2 (Store package):** desktop/wake-task,
desktop/src-tauri/src/wake.rs and desktop/msix/AppxManifest.xml. Tested on
2026-11-03: after Quit, opening Beam on the phone relaunched
`Beam.exe --minimized` in the tray with no window.

2026-11-03. Code in `spikes/ble-wake` (the app) and `spikes/ble-wake-task`
(the background task). Neither is part of the Beam build.

## Question

Can Windows start Beam when a phone's Beam Bluetooth advert appears, while
Beam isn't running at all (for example after Quit in the tray)?

## Answer: yes, with a packaged Beam and a WinRT background task

The test laptop runs Windows 11 build 26300 with an Intel Wireless
Bluetooth radio. The phone is a POCO X4 Pro 5G running Beam 0.1.7.

| Setup | Result |
|---|---|
| Win32 COM task (`SetTaskEntryPointClsid`) + `BluetoothLEAdvertisementWatcherTrigger` | **Refused**: `0x80070032 The request is not supported`. Every filter variant tried failed the same way: the full 16-byte service-data pattern, a 4-byte pattern, a service-UUID filter, and a signal-strength filter. |
| Same COM task + `TimeTrigger` (control) | Registers. The COM route works; Bluetooth triggers just aren't offered to it. |
| WinRT in-process task (`BeamWake.Task` in a DLL, hosted by Windows) + Bluetooth trigger | **Works.** It registers, fires on the phone's adverts, and `FullTrustProcessLauncher` starts the app 0.1–0.5 s after the task runs. |

Trigger details:
- The filter is one byte pattern on data type 0x21 (128-bit service data) at offset 0: Beam's service UUID, least significant byte first.
- The radio reports advertisement offload as supported, so scanning runs in the Bluetooth hardware rather than on the CPU.
- While the phone advertises, the task fired every few seconds, and again whenever the phone's random address rotated.

Measured log (ms since epoch):
- 1793700878644: registered.
- 1793700881076: the task heard `762a1b71fa9d` at -63 dBm.
- 1793700881172: the task launched the app.
- 1793700881463: the woken app logged its start.

## What this needs in Beam

1. **Package identity.** It works only with an MSIX package, as already
   planned for the Microsoft Store (`docs/DECISIONS.md`). The current MSI
   can't register background tasks. Development builds can use a loose
   registration with Developer Mode on.
2. **A small WinRT DLL in the package.** It exports
   `DllGetActivationFactory` for one class. The manifest declares:
   - `windows.activatableClass.inProcessServer`;
   - `windows.backgroundTasks` with `Task Type="bluetooth"`;
   - `desktop:windows.fullTrustProcess`, so the task can start `beam-desktop.exe`.
3. **Launch quietly.** Use `LaunchFullTrustProcessForCurrentAppWithParametersAsync`
   with a parameter group that maps to `--minimized`. If Beam is already
   running, the single-instance plugin passes the launch to it. That launch
   must not show the window, which it does today, because the trigger fires
   every few seconds.
4. **Only while Beam is closed.** Register the task when Beam quits and
   unregister it when Beam starts. Otherwise a running Beam would be
   "launched" every few seconds for nothing.
5. **Optional: trusted phones only.** The task can decode the advert
   (`core::ble::BleAdvert`) and skip phones that aren't trusted, before
   launching anything.

## Not yet measured

- Wake time from the moment Beam opens on the phone. The phone was off USB.
  The trigger's observed cadence suggests a few seconds.
- Behaviour across a reboot, on battery saver, and in Modern Standby.
- Whether the Store's certification accepts a Bluetooth background task for
  this purpose. It's a documented capability, so it should.

## Cleanup

The test package is registered as `dev.beam.WakeSpike`. Remove it with:

```powershell
Get-AppxPackage dev.beam.WakeSpike | Remove-AppxPackage
```
