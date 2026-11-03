//! Beam with no shared network: Bluetooth LE to find phones and agree on a
//! link, then a reachable LAN or laptop-hosted Wi-Fi Direct for the transfer.
//!
//! The formats and crypto are `beam_core::ble`; this module drives the
//! Windows radios through WinRT:
//!
//! - **advertising** (`BluetoothLEAdvertisementPublisher`), so phones see
//!   this laptop;
//! - **scanning** (`BluetoothLEAdvertisementWatcher`), so phones show up in
//!   Nearby even when they share no network with us;
//! - **the GATT client**: the phone always hosts the GATT service. To send
//!   to a phone we connect and ask for a link. When a phone wants to send to
//!   us it *calls* us (an advertisement naming this laptop), and we connect
//!   to it and negotiate the same LAN/Direct routes;
//! - **hosting Wi-Fi Direct**, alongside the existing Wi-Fi association.
//!   Beam never joins the phone's hotspot or restores a different SSID.
//!
//! Windows doesn't host the GATT service itself because advertising through
//! `GattServiceProvider` aborts on real hardware where a plain publisher
//! works (DECISIONS.md, "Windows doesn't host the Beam GATT service").

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use beam_core::ble::{BleDevice, ConnectResponse};
use beam_core::DeviceId;

use crate::state::AppState;

#[cfg(any(windows, test))]
mod calls;

/// A phone heard over Bluetooth.
#[derive(Debug, Clone)]
pub struct BleSighting {
    pub device: BleDevice,
    pub address: u64,
    pub random_address: bool,
    pub rssi: i16,
    pub seen: Instant,
}

/// Appends a line to Beam's log of connections with no shared network
/// (`%LOCALAPPDATA%\\Beam\\offline.log`, kept under 1 MB), so a failure on
/// real devices can be traced afterwards.
pub fn log(line: impl AsRef<str>) {
    use std::io::Write;
    let Some(dir) = dirs::data_local_dir().map(|d| d.join("Beam")) else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("offline.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1_000_000) {
        let _ = std::fs::rename(&path, dir.join("offline.old.log"));
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (h, m, s) = (secs / 3600 % 24, secs / 60 % 60, secs % 60);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{h:02}:{m:02}:{s:02} UTC {}", line.as_ref());
    }
}

/// A Bluetooth sighting is dropped after this long without hearing it again.
pub const BLE_EXPIRY: Duration = Duration::from_secs(12);

/// Our LAN IPv4 addresses.
pub fn lan_ips() -> Vec<IpAddr> {
    crate::pairing::lan_addresses()
        .into_iter()
        .filter(IpAddr::is_ipv4)
        .collect()
}

/// Where to send after a Bluetooth handshake. Dropping it releases the
/// Wi-Fi Direct group lease without changing the current Wi-Fi network.
pub struct OfflineLink {
    pub addr: SocketAddr,
    pub expect: Option<DeviceId>,
    _group: Option<GroupLease>,
}

#[cfg(windows)]
pub use win::{GroupLease, Radio};

#[cfg(not(windows))]
pub use stub::{GroupLease, Radio};

#[cfg(not(windows))]
mod stub {
    use super::*;

    pub struct GroupLease;

    pub struct Radio;

    impl Radio {
        pub async fn start(_state: Weak<AppState>) -> Result<Arc<Radio>, String> {
            Err("Bluetooth is only supported on Windows".into())
        }
        pub fn update_advertisement(&self) {}
        pub fn refresh(&self) -> Result<(), String> {
            Err("Bluetooth is only supported on Windows".into())
        }
        pub async fn connect(
            &self,
            _sighting: &BleSighting,
            _status: &(dyn Fn(&str) + Sync),
        ) -> Result<OfflineLink, String> {
            Err("Bluetooth is only supported on Windows".into())
        }
    }
}

/// The phone's answer, as text for the UI when it declined.
fn declined(response: &ConnectResponse, name: &str) -> String {
    response
        .reason
        .clone()
        .unwrap_or_else(|| format!("{name} declined"))
}

#[cfg(windows)]
mod win {
    use super::*;

    use std::collections::HashMap;
    use std::sync::Mutex;
    use tokio::sync::watch;

    use beam_core::ble::{
        self, fragment, BleAdvert, ConnectRequest, GattClientHandshake, LinkKind, Reassembler,
        PREFERRED_MTU,
    };
    #[cfg(test)]
    use windows::core::Interface;
    #[cfg(test)]
    use windows::core::HSTRING;
    use windows::core::{Ref, GUID};
    use windows::Devices::Bluetooth::Advertisement::{
        BluetoothLEAdvertisementDataSection, BluetoothLEAdvertisementPublisher,
        BluetoothLEAdvertisementPublisherStatus, BluetoothLEAdvertisementReceivedEventArgs,
        BluetoothLEAdvertisementWatcher, BluetoothLEAdvertisementWatcherStatus,
        BluetoothLEScanningMode,
    };
    use windows::Devices::Bluetooth::GenericAttributeProfile::{
        GattCharacteristic, GattClientCharacteristicConfigurationDescriptorValue,
        GattCommunicationStatus, GattSession, GattValueChangedEventArgs, GattWriteOption,
    };
    use windows::Devices::Bluetooth::{
        BluetoothAdapter, BluetoothAddressType, BluetoothCacheMode, BluetoothLEDevice,
    };
    #[cfg(test)]
    use windows::Devices::Enumeration::{DeviceInformation, DeviceInformationKind};
    use windows::Devices::Enumeration::{
        DevicePairingKinds, DevicePairingProtectionLevel, DevicePairingRequestedEventArgs,
        DevicePairingResultStatus,
    };
    use windows::Devices::WiFiDirect::{
        WiFiDirectAdvertisementListenStateDiscoverability, WiFiDirectAdvertisementPublisher,
        WiFiDirectAdvertisementPublisherStatus, WiFiDirectConfigurationMethod,
        WiFiDirectConnectionListener, WiFiDirectConnectionParameters, WiFiDirectConnectionRequest,
        WiFiDirectConnectionRequestedEventArgs, WiFiDirectConnectionStatus, WiFiDirectDevice,
        WiFiDirectPairingProcedure,
    };
    #[cfg(test)]
    use windows::Foundation::IReference;
    use windows::Foundation::TypedEventHandler;
    #[cfg(test)]
    use windows::Security::Credentials::PasswordCredential;
    use windows::Storage::Streams::{DataReader, DataWriter, IBuffer};
    #[cfg(test)]
    use windows_collections::IIterable;

    use crate::state::lock;

    type WinResult<T> = windows::core::Result<T>;

    fn win_error(message: impl AsRef<str>) -> windows::core::Error {
        windows::core::Error::new(
            windows::core::HRESULT(0x80004005_u32 as i32),
            message.as_ref(),
        )
    }

    struct OnDrop<F: FnOnce()>(Option<F>);
    impl<F: FnOnce()> Drop for OnDrop<F> {
        fn drop(&mut self) {
            if let Some(action) = self.0.take() {
                action();
            }
        }
    }

    /// WinRT Wi-Fi Direct connection operations must be CREATED on the UI
    /// thread. Their asynchronous results can then be awaited by Tokio.
    async fn on_ui<T: Send + 'static>(
        app: &tauri::AppHandle,
        action: impl FnOnce() -> WinResult<T> + Send + 'static,
    ) -> WinResult<T> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            let _ = tx.send(action());
        })
        .map_err(|e| win_error(e.to_string()))?;
        rx.await
            .map_err(|_| win_error("Beam closed while setting up Wi-Fi Direct"))?
    }

    fn guid(uuid: &str) -> GUID {
        GUID::from_u128(uuid::Uuid::parse_str(uuid).expect("a valid UUID").as_u128())
    }

    fn to_buffer(bytes: &[u8]) -> WinResult<IBuffer> {
        let writer = DataWriter::new()?;
        writer.WriteBytes(bytes)?;
        writer.DetachBuffer()
    }

    fn from_buffer(buffer: &IBuffer) -> WinResult<Vec<u8>> {
        let reader = DataReader::FromBuffer(buffer)?;
        let mut bytes = vec![0; reader.UnconsumedBufferLength()? as usize];
        reader.ReadBytes(&mut bytes)?;
        Ok(bytes)
    }

    fn text(e: windows::core::Error) -> String {
        e.message()
    }

    /// The service UUID as it appears in advertisement data (little-endian).
    fn service_uuid_le() -> [u8; 16] {
        let mut bytes = *uuid::Uuid::parse_str(ble::SERVICE_UUID)
            .expect("valid")
            .as_bytes();
        bytes.reverse();
        bytes
    }

    pub struct Radio {
        state: Weak<AppState>,
        watcher: BluetoothLEAdvertisementWatcher,
        publisher: Mutex<Option<BluetoothLEAdvertisementPublisher>>,
        /// Scan-response names by Bluetooth address (they arrive separately).
        names: Mutex<HashMap<u64, String>>,
        /// Phones whose call we're answering, by address, and since when.
        calls: Mutex<super::calls::Calls>,
        /// Accepting Wi-Fi Direct connections from phones, while needed.
        direct: Mutex<Option<DirectHost>>,
        /// Wi-Fi Direct links held for phones sending to us, and since when.
        receiving_groups: Mutex<Vec<(Instant, GroupLease)>>,
    }

    /// This laptop as a Wi-Fi Direct device that phones connect to. While a
    /// Bluetooth handshake is under way it accepts their connection
    /// requests in code (no Windows prompt) and becomes the group owner;
    /// Windows keeps its own Wi-Fi network meanwhile.
    struct DirectHost {
        id: uuid::Uuid,
        session: uuid::Uuid,
        publisher: WiFiDirectAdvertisementPublisher,
        _listener: WiFiDirectConnectionListener,
        /// Requests are accepted until then: only while a phone we're
        /// talking to over Bluetooth is expected to connect.
        accepting_until: Instant,
        /// Phones connected to us, with their Wi-Fi Direct addresses.
        devices: Vec<(WiFiDirectDevice, String)>,
        users: usize,
        cancel_pending: watch::Sender<bool>,
        /// When the last user let go.
        idle_since: Option<Instant>,
    }

    /// Keeps the Wi-Fi Direct links up while held.
    pub struct GroupLease(Weak<Radio>, uuid::Uuid);

    impl Drop for GroupLease {
        fn drop(&mut self) {
            let Some(radio) = self.0.upgrade() else {
                return;
            };
            let mut direct = lock(&radio.direct);
            if let Some(host) = direct.as_mut().filter(|host| host.id == self.1) {
                release_direct(&mut host.users, &mut host.idle_since, &host.cancel_pending);
            }
        }
    }

    /// Makes Windows forget its Wi-Fi Direct pairing with the phone at
    /// `address`. Windows keeps one after every connection, and the phone
    /// keeps the group with it; next time the phone tries to re-open that
    /// group, which fails once the laptop's Wi-Fi is on another channel
    /// ("no common channel", and MIUI asks to disconnect the phone's
    /// Wi-Fi). Without the pairing Windows answers "unknown group" instead,
    /// and Android then forgets the group and negotiates a fresh one.
    #[cfg(test)]
    pub(super) async fn forget_direct_pairing(address: &str) -> usize {
        // Wi-Fi Direct's protocol id in Windows' device association queries.
        let aqs = HSTRING::from(
            "System.Devices.Aep.ProtocolId:=\"{0407d24e-53de-4c9a-9ba1-9ced54641188}\" AND \
             System.Devices.Aep.IsPaired:=System.StructuredQueryType.Boolean#True",
        );
        let key = HSTRING::from("System.Devices.Aep.DeviceAddress");
        // Not Send: dropped before the first await.
        let op = {
            let properties = IIterable::<HSTRING>::from(vec![key.clone()]);
            DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(
                &aqs,
                &properties,
                DeviceInformationKind::AssociationEndpoint,
            )
        };
        let found = match op {
            Ok(op) => match op.await {
                Ok(found) => found,
                Err(e) => {
                    log(format!(
                        "listing Wi-Fi Direct pairings failed: {}",
                        e.message()
                    ));
                    return 0;
                }
            },
            Err(_) => return 0,
        };
        // Read everything first: the collection's iterator isn't Send.
        let paired: Vec<(DeviceInformation, String)> = found
            .into_iter()
            .map(|info| {
                // The id is "WiFiDirect#2A:F1:E2:8C:5B:65"; Windows often
                // leaves the address property empty.
                let paired_address = info
                    .Properties()
                    .and_then(|p| p.Lookup(&key))
                    .and_then(|v| v.cast::<IReference<HSTRING>>())
                    .and_then(|r| r.Value())
                    .map(|a| a.to_string().to_lowercase())
                    .ok()
                    .filter(|a| !a.is_empty())
                    .or_else(|| {
                        let id = info.Id().ok()?.to_string();
                        Some(id.split_once('#')?.1.split('_').next()?.to_lowercase())
                    })
                    .unwrap_or_default();
                (info, paired_address)
            })
            .collect();
        let mut forgotten = 0;
        for (info, paired_address) in paired {
            if !address.is_empty() && paired_address != address {
                continue;
            }
            let Ok(op) = info.Pairing().and_then(|p| p.UnpairAsync()) else {
                continue;
            };
            let result = op.await.and_then(|r| r.Status());
            log(format!(
                "forgot Wi-Fi Direct pairing with {:?} ({paired_address}): {result:?}",
                info.Name().unwrap_or_default()
            ));
            forgotten += 1;
        }
        forgotten
    }

    /// How long after a handshake starts a phone may take to connect.
    const ACCEPT_WINDOW: Duration = Duration::from_secs(90);

    /// Unused links are closed after this long (a next transfer reuses them).
    const GROUP_IDLE: Duration = Duration::from_secs(60);

    /// Offer only links that leave the existing Wi-Fi association intact.
    fn links(direct: bool) -> Vec<LinkKind> {
        if direct {
            vec![LinkKind::Lan, LinkKind::WifiDirect]
        } else {
            vec![LinkKind::Lan]
        }
    }

    fn preserve_wifi(link: Option<LinkKind>) -> Result<(), String> {
        if link == Some(LinkKind::Hotspot) {
            Err("Beam will not join the phone's hotspot because it would replace your Wi-Fi connection. Use a reachable shared Wi-Fi network or Wi-Fi Direct.".into())
        } else {
            Ok(())
        }
    }

    fn beam_characteristics<T>(mut found: HashMap<GUID, T>) -> Result<[T; 3], String> {
        let mut take = |uuid| {
            found
                .remove(&guid(uuid))
                .ok_or_else(|| "the phone's Beam service is incomplete".to_string())
        };
        Ok([
            take(ble::INFO_UUID)?,
            take(ble::CONNECT_REQUEST_UUID)?,
            take(ble::CONNECT_RESPONSE_UUID)?,
        ])
    }

    fn setup_stage(started: Instant, stage: &str) {
        log(format!(
            "setup +{} ms: {stage}",
            started.elapsed().as_millis()
        ));
    }

    fn accepts_direct(users: usize, until: Instant, now: Instant) -> bool {
        users > 0 && now < until
    }

    fn reusable_direct(users: usize, connected: bool) -> bool {
        users > 0 || connected
    }

    fn accepts_direct_session(
        current: uuid::Uuid,
        requested: uuid::Uuid,
        users: usize,
        until: Instant,
        now: Instant,
    ) -> bool {
        current == requested && accepts_direct(users, until, now)
    }

    fn release_direct(users: &mut usize, idle: &mut Option<Instant>, cancel: &watch::Sender<bool>) {
        *users = users.saturating_sub(1);
        if *users == 0 {
            *idle = Some(Instant::now());
            cancel.send_replace(true);
        }
    }

    fn direct_publisher() -> WinResult<WiFiDirectAdvertisementPublisher> {
        let publisher = WiFiDirectAdvertisementPublisher::new()?;
        let advertisement = publisher.Advertisement()?;
        advertisement.SetListenStateDiscoverability(
            WiFiDirectAdvertisementListenStateDiscoverability::Normal,
        )?;
        // Use peer provisioning. This phone's P2P interface is rejected by
        // Windows' legacy AP path even with the correct group credentials.
        advertisement.SetIsAutonomousGroupOwnerEnabled(false)?;
        advertisement.LegacySettings()?.SetIsEnabled(false)?;
        Ok(publisher)
    }

    /// The name Windows uses for this PC in Wi-Fi Direct discovery.
    fn direct_name() -> Option<String> {
        use windows::Win32::System::SystemInformation::{
            ComputerNameDnsHostname, GetComputerNameExW,
        };
        let mut buffer = [0u16; 256];
        let mut len = buffer.len() as u32;
        // SAFETY: the buffer and its length match.
        unsafe {
            GetComputerNameExW(
                ComputerNameDnsHostname,
                Some(windows::core::PWSTR(buffer.as_mut_ptr())),
                &mut len,
            )
        }
        .ok()?;
        let name = String::from_utf16_lossy(&buffer[..len as usize]);
        (!name.is_empty()).then_some(name)
    }

    impl Radio {
        /// Starts scanning and advertising.
        pub async fn start(state: Weak<AppState>) -> Result<Arc<Radio>, String> {
            let adapter = BluetoothAdapter::GetDefaultAsync()
                .map_err(text)?
                .await
                .map_err(|_| "this computer has no Bluetooth".to_string())?;
            if !adapter.IsLowEnergySupported().unwrap_or(false) {
                return Err("this computer's Bluetooth doesn't support Bluetooth LE".into());
            }
            let watcher = BluetoothLEAdvertisementWatcher::new().map_err(text)?;
            watcher
                .SetScanningMode(BluetoothLEScanningMode::Active)
                .map_err(text)?;
            let radio = Arc::new(Radio {
                state,
                watcher,
                publisher: Mutex::default(),
                names: Mutex::default(),
                calls: Mutex::default(),
                direct: Mutex::default(),
                receiving_groups: Mutex::default(),
            });
            let me = Arc::downgrade(&radio);
            radio
                .watcher
                .Received(&TypedEventHandler::new(
                    move |_, args: Ref<BluetoothLEAdvertisementReceivedEventArgs>| {
                        if let (Some(radio), Ok(args)) = (me.upgrade(), args.ok()) {
                            let _ = radio.heard(&radio, args);
                        }
                        Ok(())
                    },
                ))
                .map_err(text)?;
            radio.watcher.Start().map_err(text)?;
            radio.update_advertisement();
            radio.clone().release_groups_when_idle();
            Ok(radio)
        }

        // ---------------------------------------------------- advertising

        /// Advertises what core says for the current visibility (or stops).
        pub fn update_advertisement(&self) {
            let Some(state) = self.state.upgrade() else {
                return;
            };
            let mut current = lock(&self.publisher);
            if let Some(old) = current.take() {
                let _ = old.Stop();
            }
            let capabilities = if lan_ips().is_empty() {
                0
            } else {
                ble::caps::LAN
            };
            let Some((advert, _name)) =
                ble::advertisement(&state.device, state.visibility.mode(), capabilities)
            else {
                return;
            };
            let started = (|| -> WinResult<BluetoothLEAdvertisementPublisher> {
                let publisher = BluetoothLEAdvertisementPublisher::new()?;
                let mut payload = service_uuid_le().to_vec();
                payload.extend_from_slice(&advert.encode());
                // 0x21: service data for a 128-bit UUID.
                let section =
                    BluetoothLEAdvertisementDataSection::Create(0x21, &to_buffer(&payload)?)?;
                publisher
                    .Advertisement()?
                    .DataSections()?
                    .Append(&section)?;
                publisher.Start()?;
                Ok(publisher)
            })();
            *current = started.ok();
        }

        /// Recover after Windows stops the radios when Bluetooth is toggled.
        pub fn refresh(&self) -> Result<(), String> {
            if self.watcher.Status().map_err(text)?
                != BluetoothLEAdvertisementWatcherStatus::Started
            {
                self.watcher.Start().map_err(text)?;
            }
            let visible = self
                .state
                .upgrade()
                .is_some_and(|s| s.visibility.mode() != beam_core::VisibilityMode::Off);
            let advertising = lock(&self.publisher).as_ref().is_some_and(|p| {
                p.Status().ok() == Some(BluetoothLEAdvertisementPublisherStatus::Started)
            });
            if visible && !advertising {
                self.update_advertisement();
                if !lock(&self.publisher).as_ref().is_some_and(|p| {
                    p.Status().ok() == Some(BluetoothLEAdvertisementPublisherStatus::Started)
                }) {
                    return Err("Turn on Bluetooth to find devices without shared Wi-Fi".into());
                }
            }
            Ok(())
        }

        // ---------------------------------------------------- scanning

        fn heard(
            &self,
            me: &Arc<Radio>,
            args: &BluetoothLEAdvertisementReceivedEventArgs,
        ) -> WinResult<()> {
            let address = args.BluetoothAddress()?;
            let advertisement = args.Advertisement()?;
            let name = advertisement.LocalName()?.to_string();
            if !name.is_empty() {
                lock(&self.names).insert(address, name);
            }
            let prefix = service_uuid_le();
            let mut data = None;
            for section in advertisement.DataSections()? {
                if section.DataType()? == 0x21 {
                    let bytes = from_buffer(&section.Data()?)?;
                    if bytes.len() >= 16 + ble::ADVERT_LEN && bytes[..16] == prefix {
                        data = Some(bytes[16..].to_vec());
                    }
                }
            }
            let (Some(data), Some(state)) = (data, self.state.upgrade()) else {
                return Ok(());
            };
            let Some(advert) = BleAdvert::decode(&data) else {
                return Ok(());
            };
            let random = args.BluetoothAddressType()? == BluetoothAddressType::Random;
            if ble::is_call_for(&advert, &state.device) {
                me.clone().answer_call(address, random);
                return Ok(());
            }
            let name = lock(&self.names).get(&address).cloned();
            state.ble_seen(
                advert,
                name,
                address,
                random,
                args.RawSignalStrengthInDBm()?,
            );
            Ok(())
        }

        // ---------------------------------------------------- calls

        /// A phone called: connect to it, negotiate LAN or Wi-Fi Direct, and
        /// tell it where we receive.
        fn answer_call(self: Arc<Self>, address: u64, random: bool) {
            {
                if !lock(&self.calls).begin(address, Instant::now()) {
                    return;
                }
            }
            log(format!("call from {address:012x}"));
            tauri::async_runtime::spawn(async move {
                let result =
                    tokio::time::timeout(Duration::from_secs(120), self.take_call(address, random))
                        .await
                        .unwrap_or_else(|_| Err("connection setup timed out".into()));
                lock(&self.calls).finish(address, Instant::now());
                if let Err(problem) = result {
                    log(format!("call from {address:012x} failed: {problem}"));
                    if let Some(state) = self.state.upgrade() {
                        state
                            .show_error(format!("A phone tried to send over Bluetooth: {problem}"));
                    }
                }
            });
        }

        async fn take_call(self: &Arc<Self>, address: u64, random: bool) -> Result<(), String> {
            let started = Instant::now();
            let state = self.state.upgrade().ok_or("Beam is closing")?;
            let device = Self::device(address, random).await?;
            setup_stage(started, "receiving: Bluetooth device ready");
            let close = device.clone();
            let _device = OnDrop(Some(move || {
                let _ = close.Close();
            }));
            let direct = self.listen_direct().await;
            setup_stage(started, "receiving: Wi-Fi Direct offer ready");
            log(format!(
                "answering: Wi-Fi Direct as {:?}",
                direct.as_ref().map(|(p, _)| &p.name)
            ));
            let request = ConnectRequest {
                caps: 0,
                links: links(direct.is_some()),
                lan: lan_ips()
                    .iter()
                    .map(|ip| SocketAddr::new(*ip, state.port()).to_string())
                    .collect(),
                hotspot: None,
                port: Some(state.port()),
                direct: direct.as_ref().map(|(peer, _)| peer.clone()),
            };
            let response = Self::handshake(&device, &request).await.map_err(text);
            let _ = device.Close();
            let response = response??;
            setup_stage(started, "receiving: phone replied");
            preserve_wifi(response.link)?;
            log(format!(
                "phone answered: {:?} {:?} {:?}",
                response.link, response.addr, response.reason
            ));
            match response.link {
                // On one network: the phone sends to our LAN address.
                Some(LinkKind::Lan) => Ok(()),
                // The phone connected over Wi-Fi Direct and sends to us there.
                Some(LinkKind::WifiDirect) => {
                    let (_, lease) =
                        direct.ok_or("the phone chose Wi-Fi Direct, which we didn't offer")?;
                    lock(&self.receiving_groups).push((Instant::now(), lease));
                    Ok(())
                }
                _ => Err(declined(&response, "the phone")),
            }
        }

        /// Releases unused private Wi-Fi Direct groups.
        fn release_groups_when_idle(self: Arc<Self>) {
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    let Some(state) = self.state.upgrade() else {
                        return;
                    };
                    self.release_disconnected_groups();
                    if !state.is_receiving() {
                        // Leave time for the transfer to start before letting go.
                        lock(&self.receiving_groups)
                            .retain(|(since, _)| since.elapsed() < Duration::from_secs(90));
                    }
                    let transferring = state.is_transferring();
                    let mut direct = lock(&self.direct);
                    let unused = direct.as_ref().is_some_and(|host| {
                        !transferring && host.idle_since.is_some_and(|t| t.elapsed() >= GROUP_IDLE)
                    });
                    if unused {
                        if let Some(host) = direct.take() {
                            let _ = host.publisher.Stop();
                            // Release native handles after the idle window;
                            // this never disconnects the infrastructure STA.
                            for (device, _) in host.devices {
                                let _ = device.Close();
                            }
                        }
                    }
                }
            });
        }

        // ---------------------------------------------------- sending

        /// Asks a phone heard over Bluetooth for LAN or Wi-Fi Direct.
        pub async fn connect(
            self: &Arc<Self>,
            sighting: &BleSighting,
            status: &(dyn Fn(&str) + Sync),
        ) -> Result<OfflineLink, String> {
            let started = Instant::now();
            let name = &sighting.device.name;
            status("Connecting over Bluetooth…");
            let device = Self::device(sighting.address, sighting.random_address).await?;
            setup_stage(started, "sending: Bluetooth device ready");
            let close = device.clone();
            let _device = OnDrop(Some(move || {
                let _ = close.Close();
            }));
            let direct = self.listen_direct().await;
            setup_stage(started, "sending: Wi-Fi Direct offer ready");
            log(format!(
                "sending to {name}: Wi-Fi Direct as {:?}",
                direct.as_ref().map(|(p, _)| &p.name)
            ));
            let request = ConnectRequest {
                caps: 0,
                links: links(direct.is_some()),
                lan: lan_ips()
                    .iter()
                    .map(|ip| {
                        SocketAddr::new(*ip, self.state.upgrade().map_or(0, |s| s.port()))
                            .to_string()
                    })
                    .collect(),
                hotspot: None,
                port: None,
                direct: direct.as_ref().map(|(peer, _)| peer.clone()),
            };
            status(&format!("Waiting for {name} to connect…"));
            let response = Self::handshake(&device, &request).await.map_err(text);
            let _ = device.Close();
            if let Err(e) = &response {
                log(format!("handshake with {name} failed: {e}"));
            }
            let response = response??;
            setup_stage(started, "sending: phone replied");
            preserve_wifi(response.link)?;
            log(format!(
                "{name} answered: {:?} {:?} {:?}",
                response.link, response.addr, response.reason
            ));
            let expect = sighting.device.device_id;
            match response.link {
                Some(LinkKind::Lan) => Ok(OfflineLink {
                    addr: response
                        .addr
                        .as_deref()
                        .ok_or("the phone didn't say where to connect")?
                        .parse()
                        .map_err(|_| "the phone sent an unreadable address".to_string())?,
                    expect,
                    _group: None,
                }),
                // The phone connected over Wi-Fi Direct: it receives at its
                // address there.
                Some(LinkKind::WifiDirect) => {
                    let (_, lease) =
                        direct.ok_or("the phone chose Wi-Fi Direct, which we didn't offer")?;
                    Ok(OfflineLink {
                        addr: response
                            .addr
                            .as_deref()
                            .ok_or("the phone didn't say where to connect")?
                            .parse()
                            .map_err(|_| "the phone sent an unreadable address".to_string())?,
                        expect,
                        _group: Some(lease),
                    })
                }
                _ => Err(declined(&response, name)),
            }
        }

        // ---------------------------------------------------- Wi-Fi Direct

        /// Accepts Wi-Fi Direct connections for the next minute (starting the
        /// listener if needed). `None` when this PC can't do Wi-Fi Direct.
        async fn listen_direct(self: &Arc<Self>) -> Option<(ble::DirectPeer, GroupLease)> {
            let state = self.state.upgrade()?;
            let radio = self.clone();
            match on_ui(&state.app, move || Ok(radio.listen_direct_on_ui())).await {
                Ok(result) => result,
                Err(e) => {
                    log(format!("Wi-Fi Direct unavailable: {}", e.message()));
                    None
                }
            }
        }

        fn listen_direct_on_ui(self: &Arc<Self>) -> Option<(ble::DirectPeer, GroupLease)> {
            let name = direct_name()?;
            self.release_disconnected_groups();
            let mut direct = lock(&self.direct);
            let running = direct.as_ref().is_some_and(|host| {
                reusable_direct(
                    host.users,
                    host.devices.iter().any(|(device, _)| {
                        device.ConnectionStatus().ok()
                            == Some(WiFiDirectConnectionStatus::Connected)
                    }),
                ) && host.publisher.Status().ok()
                    == Some(WiFiDirectAdvertisementPublisherStatus::Started)
            });
            if !running {
                if let Some(old) = direct.take() {
                    log("retiring unused Wi-Fi Direct group before a fresh setup");
                    old.cancel_pending.send_replace(true);
                    let _ = old.publisher.Stop();
                    for (device, _) in old.devices {
                        let _ = device.Close();
                    }
                }
                *direct = Some(self.start_direct().ok()?);
            }
            let host = direct.as_mut()?;
            host.accepting_until = Instant::now() + ACCEPT_WINDOW;
            if host.users == 0 {
                // Pending requests from the previous transfer keep their
                // cancelled receiver; a new transfer gets a fresh window.
                host.cancel_pending = watch::channel(false).0;
                host.session = uuid::Uuid::new_v4();
            }
            host.users += 1;
            host.idle_since = None;
            Some((
                ble::DirectPeer {
                    name,
                    network_name: None,
                    passphrase: None,
                },
                GroupLease(Arc::downgrade(self), host.id),
            ))
        }

        fn pending_direct(&self, id: uuid::Uuid) -> Option<(uuid::Uuid, watch::Receiver<bool>)> {
            let direct = lock(&self.direct);
            let host = direct.as_ref().filter(|host| {
                host.id == id && accepts_direct(host.users, host.accepting_until, Instant::now())
            })?;
            Some((host.session, host.cancel_pending.subscribe()))
        }

        fn release_disconnected_groups(&self) {
            if self
                .state
                .upgrade()
                .is_some_and(|state| state.is_receiving())
            {
                return;
            }
            let disconnected = lock(&self.direct)
                .as_ref()
                .filter(|host| {
                    !host.devices.is_empty()
                        && host.devices.iter().all(|(device, _)| {
                            device.ConnectionStatus().ok()
                                == Some(WiFiDirectConnectionStatus::Disconnected)
                        })
                })
                .map(|host| host.id);
            if let Some(id) = disconnected {
                // The phone has already left. Keeping these receive leases
                // for the setup grace period would reuse its inactive WPS group.
                lock(&self.receiving_groups).retain(|(_, lease)| lease.1 != id);
            }
        }

        fn active_direct_session(&self, id: uuid::Uuid, session: uuid::Uuid) -> bool {
            lock(&self.direct).as_ref().is_some_and(|host| {
                host.id == id
                    && accepts_direct_session(
                        host.session,
                        session,
                        host.users,
                        host.accepting_until,
                        Instant::now(),
                    )
            })
        }

        fn start_direct(self: &Arc<Self>) -> WinResult<DirectHost> {
            let publisher = direct_publisher()?;
            let host_id = uuid::Uuid::new_v4();
            let listener = WiFiDirectConnectionListener::new()?;
            let me = Arc::downgrade(self);
            listener.ConnectionRequested(&TypedEventHandler::new(
                move |_, args: Ref<WiFiDirectConnectionRequestedEventArgs>| {
                    let (Some(radio), Ok(args)) = (me.upgrade(), args.ok()) else {
                        return Ok(());
                    };
                    let request = args.GetConnectionRequest()?;
                    let pending = radio.pending_direct(host_id);
                    log(format!(
                        "Wi-Fi Direct request (expected: {})",
                        pending.is_some()
                    ));
                    if let Some((session, cancel)) = pending {
                        tauri::async_runtime::spawn(async move {
                            match radio.accept_direct(request, host_id, session, cancel).await {
                                Ok(()) => log("Wi-Fi Direct connected"),
                                Err(e) => {
                                    log(format!("Wi-Fi Direct accept failed: {}", e.message()))
                                }
                            }
                        });
                    } else {
                        // No live transfer owns this request. Close it now.
                        let _ = request.Close();
                    }
                    Ok(())
                },
            ))?;
            publisher.Start()?;
            log(format!(
                "Wi-Fi Direct publisher (negotiated group): {:?}",
                publisher.Status()
            ));
            Ok(DirectHost {
                id: host_id,
                session: uuid::Uuid::new_v4(),
                publisher,
                _listener: listener,
                accepting_until: Instant::now(),
                devices: Vec::new(),
                users: 0,
                cancel_pending: watch::channel(true).0,
                idle_since: None,
            })
        }

        /// Accepts a phone's request: push-button pairing handled here, so
        /// Windows shows nothing, then the connection, kept while in use.
        async fn accept_direct(
            self: Arc<Self>,
            request: WiFiDirectConnectionRequest,
            host_id: uuid::Uuid,
            session: uuid::Uuid,
            mut cancel: watch::Receiver<bool>,
        ) -> WinResult<()> {
            let close = request.clone();
            let _request = OnDrop(Some(move || {
                let _ = close.Close();
            }));
            tokio::select! {
                biased;
                _ = cancel.wait_for(|cancelled| *cancelled) => Err(win_error("Wi-Fi Direct transfer setup ended; pairing cancelled")),
                result = self.accept_direct_inner(request, host_id, session) => result,
            }
        }

        async fn accept_direct_inner(
            self: Arc<Self>,
            request: WiFiDirectConnectionRequest,
            host_id: uuid::Uuid,
            session: uuid::Uuid,
        ) -> WinResult<()> {
            if !self.active_direct_session(host_id, session) {
                return Err(win_error("Wi-Fi Direct request expired"));
            }
            let info = request.DeviceInformation()?;
            let id = info.Id()?;
            // "WiFiDirect#2A:F1:E2:8C:5B:65_PendingRequest": the phone's address.
            let address = id
                .to_string()
                .split_once('#')
                .map(|(_, rest)| rest.split('_').next().unwrap_or(rest).to_lowercase())
                .unwrap_or_default();
            let pairing = info.Pairing()?;
            let already = pairing.IsPaired().unwrap_or(false);
            log(format!(
                "request from {} ({id}), paired already: {already}",
                info.Name()?
            ));
            // A phone that connected before stays paired (Wi-Fi Direct keeps a
            // persistent group): pairing again stalls, so connect straight away.
            if !already {
                self.pair_direct(&info).await?;
            }
            if !self.active_direct_session(host_id, session) {
                return Err(win_error("Wi-Fi Direct request expired after pairing"));
            }
            let state = self
                .state
                .upgrade()
                .ok_or_else(|| win_error("Beam is closing"))?;
            let (operation, _cancel) = on_ui(&state.app, move || {
                let operation = WiFiDirectDevice::FromIdAsync(&id)?;
                let cancel = operation.clone();
                Ok((
                    operation,
                    OnDrop(Some(move || {
                        let _ = cancel.Cancel();
                    })),
                ))
            })
            .await?;
            let device = match tokio::time::timeout(Duration::from_secs(25), operation)
                .await
                .map_err(|_| win_error("Wi-Fi Direct connection timed out"))?
            {
                Ok(device) => device,
                Err(e) => {
                    log(format!(
                        "connecting failed ({:?}): {}",
                        e.code(),
                        e.message()
                    ));
                    return Err(e);
                }
            };
            log(format!("connected: {:?}", device.ConnectionStatus()));
            if let Some(host) = lock(&self.direct).as_mut().filter(|host| {
                host.id == host_id
                    && accepts_direct_session(
                        host.session,
                        session,
                        host.users,
                        host.accepting_until,
                        Instant::now(),
                    )
            }) {
                host.devices.push((device, address));
            } else {
                let _ = device.Close();
                return Err(win_error("Wi-Fi Direct request expired before connecting"));
            }
            Ok(())
        }

        /// Handle push-button confirmation; Windows may still require system consent.
        async fn pair_direct(
            &self,
            info: &windows::Devices::Enumeration::DeviceInformation,
        ) -> WinResult<()> {
            let state = self
                .state
                .upgrade()
                .ok_or_else(|| win_error("Beam is closing"))?;
            let info = info.clone();
            let (_handler, _cancel, operation) = on_ui(&state.app, move || {
                log(format!(
                    "Wi-Fi Direct pairing begin: can_pair={:?}, is_paired={:?}",
                    info.Pairing().and_then(|p| p.CanPair()),
                    info.Pairing().and_then(|p| p.IsPaired())
                ));
                let params = WiFiDirectConnectionParameters::new()?;
                // Prefer the laptop as owner while allowing normal channel
                // negotiation. Android requests the client role (intent 0).
                params.SetGroupOwnerIntent(14)?;
                params
                    .PreferenceOrderedConfigurationMethods()?
                    .Append(WiFiDirectConfigurationMethod::PushButton)?;
                params.SetPreferredPairingProcedure(WiFiDirectPairingProcedure::Invitation)?;
                log("Wi-Fi Direct pairing procedure: accepting device selects push-button");
                let custom = info.Pairing()?.Custom()?;
                let token = custom.PairingRequested(&TypedEventHandler::new(
                    |_, args: Ref<DevicePairingRequestedEventArgs>| {
                        let args = args.ok()?;
                        let kind = args.PairingKind()?;
                        log(format!("Wi-Fi Direct pairing ceremony: {kind:?}"));
                        if kind == DevicePairingKinds::ConfirmOnly {
                            args.Accept()?;
                            log("Wi-Fi Direct push-button confirmation accepted");
                        } else {
                            log("Wi-Fi Direct pairing requested an unsupported ceremony");
                        }
                        Ok(())
                    },
                ))?;
                let cleanup = custom.clone();
                let handler = OnDrop(Some(move || {
                    let _ = cleanup.RemovePairingRequested(token);
                }));
                let operation = custom.PairWithProtectionLevelAndSettingsAsync(
                    DevicePairingKinds::ConfirmOnly,
                    DevicePairingProtectionLevel::Default,
                    &params,
                )?;
                log("Wi-Fi Direct pairing operation created");
                let cancel = operation.clone();
                Ok((
                    handler,
                    OnDrop(Some(move || {
                        let _ = cancel.Cancel();
                    })),
                    operation,
                ))
            })
            .await?;
            let paired = tokio::time::timeout(Duration::from_secs(25), operation)
                .await
                .map_err(|_| win_error("Wi-Fi Direct pairing timed out"))?;
            match paired {
                Ok(result) => {
                    let status = result.Status()?;
                    log(format!("pairing: {status:?}"));
                    if status == DevicePairingResultStatus::Paired
                        || status == DevicePairingResultStatus::AlreadyPaired
                    {
                        Ok(())
                    } else {
                        Err(win_error(format!(
                            "Windows refused Wi-Fi Direct pairing ({status:?})"
                        )))
                    }
                }
                Err(e) => {
                    log(format!("pairing failed ({:?}): {}", e.code(), e.message()));
                    Err(e)
                }
            }
        }

        async fn device(address: u64, random: bool) -> Result<BluetoothLEDevice, String> {
            let kind = if random {
                BluetoothAddressType::Random
            } else {
                BluetoothAddressType::Public
            };
            BluetoothLEDevice::FromBluetoothAddressWithBluetoothAddressTypeAsync(address, kind)
                .map_err(text)?
                .await
                .map_err(|_| "the phone is out of Bluetooth range".to_string())
        }

        /// Reads the phone's `INFO`, writes `request`, and waits for its
        /// `CONNECT_RESPONSE`.
        async fn handshake(
            device: &BluetoothLEDevice,
            request: &ConnectRequest,
        ) -> WinResult<Result<ConnectResponse, String>> {
            let started = Instant::now();
            let session = GattSession::FromDeviceIdAsync(&device.BluetoothDeviceId()?)?.await?;
            let _ = session.SetMaintainConnection(true);
            let cleanup = session.clone();
            let _session = OnDrop(Some(move || {
                let _ = cleanup.SetMaintainConnection(false);
            }));
            let services = device
                .GetGattServicesForUuidWithCacheModeAsync(
                    guid(ble::SERVICE_UUID),
                    BluetoothCacheMode::Uncached,
                )?
                .await?;
            let service_status = services.Status()?;
            let service_count = services.Services()?.Size()?;
            log(format!(
                "BLE service query: {service_status:?}, count={service_count}, connection={:?}",
                device.ConnectionStatus()
            ));
            if service_status != GattCommunicationStatus::Success || service_count == 0 {
                return Ok(Err("Couldn't reach the phone's Beam Bluetooth service. Keep Beam open on the phone and retry.".into()));
            }
            let service = services.Services()?.GetAt(0)?;
            setup_stage(started, "BLE: service discovered");
            // One fresh discovery returns all three characteristics. Separate
            // UUID queries repeat the same on-air discovery round trips.
            let result = service
                .GetCharacteristicsWithCacheModeAsync(BluetoothCacheMode::Uncached)?
                .await?;
            if result.Status()? != GattCommunicationStatus::Success {
                return Ok(Err("couldn't discover the phone's Beam service".into()));
            }
            let mut found = HashMap::new();
            for characteristic in result.Characteristics()? {
                found.insert(characteristic.Uuid()?, characteristic);
            }
            let [info_char, request_char, response_char]: [GattCharacteristic; 3] =
                match beam_characteristics(found) {
                    Ok(found) => found,
                    Err(problem) => return Ok(Err(problem)),
                };
            setup_stage(started, "BLE: characteristics discovered");
            let mtu = (session.MaxPduSize().unwrap_or(23) as usize).min(PREFERRED_MTU);

            let read = info_char
                .ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached)?
                .await?;
            if read.Status()? != GattCommunicationStatus::Success {
                return Ok(Err("couldn't read the phone's Beam info".into()));
            }
            let info = from_buffer(&read.Value()?)?;
            setup_stage(started, "BLE: peer info read");

            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
            let token = response_char.ValueChanged(&TypedEventHandler::new(
                move |_, args: Ref<GattValueChangedEventArgs>| {
                    if let Ok(args) = args.ok() {
                        if let Ok(bytes) = from_buffer(&args.CharacteristicValue()?) {
                            let _ = tx.send(bytes);
                        }
                    }
                    Ok(())
                },
            ))?;
            let cleanup = response_char.clone();
            let _notification = OnDrop(Some(move || {
                let _ = cleanup.RemoveValueChanged(token);
            }));
            let subscribed = response_char
                .WriteClientCharacteristicConfigurationDescriptorAsync(
                    GattClientCharacteristicConfigurationDescriptorValue::Notify,
                )?
                .await?;
            if subscribed != GattCommunicationStatus::Success {
                return Ok(Err("couldn't subscribe to the phone's answer".into()));
            }

            let (_, data, opener) = match GattClientHandshake::request(&info, request) {
                Ok(started) => started,
                Err(e) => return Ok(Err(e.to_string())),
            };
            for part in fragment(&data, mtu).unwrap_or_default() {
                let write = request_char.WriteValueWithResultAndOptionAsync(
                    &to_buffer(&part)?,
                    GattWriteOption::WriteWithResponse,
                )?;
                let written = write.await?;
                if written.Status()? != GattCommunicationStatus::Success {
                    return Ok(Err("the phone didn't take the request".into()));
                }
            }
            setup_stage(started, "BLE: link request sent");
            // The phone establishes the private link before answering.
            let mut reassembler = Reassembler::default();
            let whole = tokio::time::timeout(Duration::from_secs(90), async {
                while let Some(part) = rx.recv().await {
                    if let Ok(Some(whole)) = reassembler.push(&part) {
                        return Some(whole);
                    }
                }
                None
            })
            .await;
            let _ = response_char.RemoveValueChanged(token);
            let _ = session.SetMaintainConnection(false);
            let Ok(Some(whole)) = whole else {
                return Ok(Err("the phone didn't answer over Bluetooth".into()));
            };
            setup_stage(started, "BLE: link response received");
            Ok(opener.open(&whole).map_err(|e| e.to_string()))
        }
    }

    /// Run on a machine with Bluetooth LE: `cargo test -p beam-desktop -- --ignored`.
    #[cfg(test)]
    mod hardware {
        use super::*;

        #[test]
        fn offers_never_ask_the_phone_to_replace_the_laptops_wifi() {
            assert_eq!(links(true), vec![LinkKind::Lan, LinkKind::WifiDirect]);
            assert_eq!(links(false), vec![LinkKind::Lan]);
        }

        #[test]
        fn an_unsolicited_hotspot_response_is_rejected_before_joining() {
            assert!(preserve_wifi(Some(LinkKind::Hotspot))
                .unwrap_err()
                .contains("Wi-Fi"));
            assert!(preserve_wifi(Some(LinkKind::Lan)).is_ok());
            assert!(preserve_wifi(Some(LinkKind::WifiDirect)).is_ok());
            assert!(preserve_wifi(None).is_ok());
        }

        #[test]
        fn one_discovery_result_selects_required_characteristics_by_uuid() {
            let found = HashMap::from([
                (guid(ble::CONNECT_RESPONSE_UUID), 3),
                (guid(ble::INFO_UUID), 1),
                (GUID::zeroed(), 9),
                (guid(ble::CONNECT_REQUEST_UUID), 2),
            ]);
            assert_eq!(beam_characteristics(found).unwrap(), [1, 2, 3]);
        }

        #[test]
        fn an_incomplete_discovery_result_cannot_start_a_handshake() {
            let found = HashMap::from([(guid(ble::INFO_UUID), 1)]);
            assert!(beam_characteristics(found).is_err());
        }

        #[test]
        fn a_phone_reply_after_setup_finished_is_not_accepted() {
            let now = Instant::now();
            let deadline = now + ACCEPT_WINDOW;
            assert!(accepts_direct(1, deadline, now));
            assert!(!accepts_direct(0, deadline, now + Duration::from_secs(40)));
            assert!(!accepts_direct(1, deadline, deadline));
        }

        #[test]
        fn a_connected_idle_group_is_reusable_but_an_empty_group_is_not() {
            assert!(
                reusable_direct(0, true),
                "the next sender would close a connected group"
            );
            assert!(reusable_direct(1, false));
            assert!(!reusable_direct(0, false));
        }

        #[test]
        fn a_ready_old_pairing_cannot_attach_to_a_fast_retry() {
            let now = Instant::now();
            let old_session = uuid::Uuid::new_v4();
            let retry = uuid::Uuid::new_v4();
            assert!(accepts_direct_session(
                retry,
                retry,
                1,
                now + ACCEPT_WINDOW,
                now
            ));
            assert!(!accepts_direct_session(
                retry,
                old_session,
                1,
                now + ACCEPT_WINDOW,
                now
            ));
        }

        #[tokio::test]
        async fn ending_the_last_transfer_cancels_pending_pairing() {
            let (cancel, mut pending) = watch::channel(false);
            let mut users = 2;
            let mut idle = None;
            release_direct(&mut users, &mut idle, &cancel);
            assert!(!*pending.borrow());
            assert!(idle.is_none());
            release_direct(&mut users, &mut idle, &cancel);
            assert!(idle.is_some());
            tokio::time::timeout(
                Duration::from_millis(100),
                pending.wait_for(|cancelled| *cancelled),
            )
            .await
            .expect("pending pairing survived the last transfer")
            .unwrap();
        }

        #[tokio::test]
        #[ignore = "starts negotiated discovery on a Wi-Fi Direct capable PC; no peer connection"]
        async fn negotiated_discovery_preserves_existing_wifi() {
            use windows::Networking::Connectivity::{NetworkConnectivityLevel, NetworkInformation};
            fn connected_wifi() -> Vec<String> {
                let mut names: Vec<_> = NetworkInformation::GetConnectionProfiles()
                    .unwrap()
                    .into_iter()
                    .filter(|p| {
                        p.IsWlanConnectionProfile().unwrap_or(false)
                            && p.GetNetworkConnectivityLevel().unwrap()
                                != NetworkConnectivityLevel::None
                    })
                    .map(|p| p.ProfileName().unwrap().to_string())
                    .collect();
                names.sort();
                names
            }
            let before = connected_wifi();
            assert!(
                !before.is_empty(),
                "this check needs an existing infrastructure Wi-Fi connection"
            );
            let publisher = direct_publisher().unwrap();
            let cleanup = publisher.clone();
            let _stop = OnDrop(Some(move || {
                let _ = cleanup.Stop();
            }));
            publisher.Start().unwrap();
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let after = connected_wifi();
            assert!(
                before.iter().all(|name| after.contains(name)),
                "the group replaced an existing Wi-Fi connection"
            );
            assert!(
                !publisher
                    .Advertisement()
                    .unwrap()
                    .IsAutonomousGroupOwnerEnabled()
                    .unwrap(),
                "peer provisioning must negotiate the group"
            );
            let legacy = publisher.Advertisement().unwrap().LegacySettings().unwrap();
            assert!(
                !legacy.IsEnabled().unwrap(),
                "P2P peers must not be sent through the legacy AP path"
            );
            assert_eq!(
                publisher.Status().unwrap(),
                WiFiDirectAdvertisementPublisherStatus::Started
            );
        }

        #[tokio::test]
        #[ignore = "needs a Bluetooth LE adapter"]
        async fn the_publisher_advertises_beam_service_data() {
            let publisher = BluetoothLEAdvertisementPublisher::new().unwrap();
            let mut payload = service_uuid_le().to_vec();
            payload.extend([1u8, 1, 2, 3, 4, 5, 6, 7, 0, 0]);
            let section =
                BluetoothLEAdvertisementDataSection::Create(0x21, &to_buffer(&payload).unwrap())
                    .unwrap();
            publisher
                .Advertisement()
                .unwrap()
                .DataSections()
                .unwrap()
                .Append(&section)
                .unwrap();
            publisher.Start().unwrap();
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let status = publisher.Status().unwrap();
            publisher.Stop().unwrap();
            assert_eq!(
                status,
                windows::Devices::Bluetooth::Advertisement::BluetoothLEAdvertisementPublisherStatus::Started
            );
        }

        /// Lists the Beam advertisements in range, for debugging on real
        /// devices: `cargo test -p beam-desktop beam_adverts_in_range -- --ignored --nocapture`.
        #[tokio::test]
        #[ignore = "needs a Bluetooth LE adapter and a Beam device nearby"]
        async fn beam_adverts_in_range() {
            let watcher = BluetoothLEAdvertisementWatcher::new().unwrap();
            watcher
                .SetScanningMode(BluetoothLEScanningMode::Active)
                .unwrap();
            // (address, service data) -> (signal, name)
            type Heard = HashMap<(u64, Vec<u8>), (i16, String)>;
            let heard: Arc<Mutex<Heard>> = Arc::default();
            let total = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let (sink, count) = (heard.clone(), total.clone());
            watcher
                .Received(&TypedEventHandler::new(
                    move |_, args: Ref<BluetoothLEAdvertisementReceivedEventArgs>| {
                        count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Ok(args) = args.ok() else { return Ok(()) };
                        let advertisement = args.Advertisement()?;
                        for section in advertisement.DataSections()? {
                            if section.DataType()? != 0x21 {
                                continue;
                            }
                            let bytes = from_buffer(&section.Data()?)?;
                            if bytes.len() >= 16 && bytes[..16] == service_uuid_le() {
                                sink.lock().unwrap().insert(
                                    (args.BluetoothAddress()?, bytes[16..].to_vec()),
                                    (
                                        args.RawSignalStrengthInDBm()?,
                                        advertisement.LocalName()?.to_string(),
                                    ),
                                );
                            }
                        }
                        Ok(())
                    },
                ))
                .unwrap();
            watcher.Start().unwrap();
            tokio::time::sleep(Duration::from_secs(15)).await;
            watcher.Stop().unwrap();
            println!(
                "{} advertisements in 15 s; Beam ones:",
                total.load(std::sync::atomic::Ordering::Relaxed)
            );
            for ((address, data), (rssi, name)) in heard.lock().unwrap().iter() {
                println!(
                    "  {address:012x} rssi {rssi} name {name:?} data {} -> {:?}",
                    hex::encode(data),
                    BleAdvert::decode(data)
                );
            }
        }

        /// Waits for a phone's call, then does what answering does (connect,
        /// GATT handshake) and prints each step, without joining its Wi-Fi:
        /// `cargo test -p beam-desktop answer_a_call_step_by_step -- --ignored --nocapture`.
        #[tokio::test]
        #[ignore = "needs a phone calling this laptop"]
        async fn answer_a_call_step_by_step() {
            let watcher = BluetoothLEAdvertisementWatcher::new().unwrap();
            watcher
                .SetScanningMode(BluetoothLEScanningMode::Active)
                .unwrap();
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            watcher
                .Received(&TypedEventHandler::new(
                    move |_, args: Ref<BluetoothLEAdvertisementReceivedEventArgs>| {
                        let Ok(args) = args.ok() else { return Ok(()) };
                        for section in args.Advertisement()?.DataSections()? {
                            let bytes = from_buffer(&section.Data()?)?;
                            if section.DataType()? == 0x21
                                && bytes.len() >= 16 + ble::ADVERT_LEN
                                && bytes[..16] == service_uuid_le()
                            {
                                if let Some(advert) = BleAdvert::decode(&bytes[16..]) {
                                    if advert.flags & ble::caps::CALL != 0 {
                                        let random = args.BluetoothAddressType()?
                                            == BluetoothAddressType::Random;
                                        let _ = tx.send((args.BluetoothAddress()?, random, advert));
                                    }
                                }
                            }
                        }
                        Ok(())
                    },
                ))
                .unwrap();
            watcher.Start().unwrap();
            println!("waiting up to 5 minutes for a call…");
            let (address, random, advert) =
                tokio::time::timeout(Duration::from_secs(300), rx.recv())
                    .await
                    .expect("no call heard")
                    .unwrap();
            watcher.Stop().unwrap();
            let started = Instant::now();
            println!("call from {address:012x} (random {random}): {advert:?}");
            let device = Radio::device(address, random).await;
            println!(
                "{:?} device: {:?}",
                started.elapsed(),
                device.as_ref().map(|_| "ok")
            );
            let device = device.unwrap();
            for mode in [BluetoothCacheMode::Uncached, BluetoothCacheMode::Cached] {
                let services = device
                    .GetGattServicesWithCacheModeAsync(mode)
                    .unwrap()
                    .await
                    .unwrap();
                println!(
                    "{:?} services ({mode:?}): {:?}",
                    started.elapsed(),
                    services.Status()
                );
                for service in services.Services().into_iter().flatten() {
                    let uuid = service.Uuid().unwrap();
                    let chars = service
                        .GetCharacteristicsWithCacheModeAsync(mode)
                        .unwrap()
                        .await
                        .unwrap();
                    println!(
                        "  service {uuid:?}: characteristics {:?} {:?}",
                        chars.Status(),
                        chars
                            .Characteristics()
                            .into_iter()
                            .flatten()
                            .map(|c| c.Uuid().unwrap())
                            .collect::<Vec<_>>()
                    );
                }
            }
            let request = ConnectRequest {
                caps: 0,
                links: vec![LinkKind::Lan, LinkKind::Hotspot],
                lan: lan_ips().iter().map(ToString::to_string).collect(),
                hotspot: None,
                port: Some(9000),
                direct: None,
            };
            let response = Radio::handshake(&device, &request).await;
            println!("{:?} handshake: {response:?}", started.elapsed());
            let _ = device.Close();
        }

        /// Advertises over Wi-Fi Direct and accepts incoming connection
        /// requests in code (no Windows pairing prompt), printing each
        /// step, for BEAM_HOLD_SECS (default 180). BEAM_AUTONOMOUS=1 makes
        /// this laptop a group owner up front:
        /// `cargo test -p beam-desktop accept_p2p_connections -- --ignored --nocapture`.
        #[tokio::test]
        #[ignore = "needs a Wi-Fi Direct capable adapter and a phone"]
        async fn accept_p2p_connections() {
            use windows::Devices::Enumeration::{
                DeviceInformationCustomPairing, DevicePairingKinds, DevicePairingProtectionLevel,
                DevicePairingRequestedEventArgs,
            };
            use windows::Devices::WiFiDirect::{
                WiFiDirectAdvertisementListenStateDiscoverability,
                WiFiDirectAdvertisementPublisher, WiFiDirectConfigurationMethod,
                WiFiDirectConnectionListener, WiFiDirectConnectionParameters,
                WiFiDirectConnectionRequest, WiFiDirectConnectionRequestedEventArgs,
                WiFiDirectDevice, WiFiDirectPairingProcedure,
            };
            let hold: u64 = std::env::var("BEAM_HOLD_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(180);
            let autonomous = std::env::var("BEAM_AUTONOMOUS").is_ok_and(|v| v == "1");
            let publisher = WiFiDirectAdvertisementPublisher::new().unwrap();
            let advertisement = publisher.Advertisement().unwrap();
            advertisement
                .SetListenStateDiscoverability(
                    WiFiDirectAdvertisementListenStateDiscoverability::Normal,
                )
                .unwrap();
            advertisement
                .SetIsAutonomousGroupOwnerEnabled(autonomous)
                .unwrap();
            let listener = WiFiDirectConnectionListener::new().unwrap();
            let (tx, mut rx) =
                tokio::sync::mpsc::unbounded_channel::<WiFiDirectConnectionRequest>();
            listener
                .ConnectionRequested(&TypedEventHandler::new(
                    move |_, args: Ref<WiFiDirectConnectionRequestedEventArgs>| {
                        if let Ok(args) = args.ok() {
                            let _ = tx.send(args.GetConnectionRequest()?);
                        }
                        Ok(())
                    },
                ))
                .unwrap();
            publisher.Start().unwrap();
            println!(
                "advertising (autonomous {autonomous}): {:?}",
                publisher.Status()
            );
            let deadline = Instant::now() + Duration::from_secs(hold);
            let mut devices = Vec::new();
            while let Ok(Some(request)) = tokio::time::timeout(
                deadline.saturating_duration_since(Instant::now()),
                rx.recv(),
            )
            .await
            {
                let started = Instant::now();
                let info = request.DeviceInformation().unwrap();
                println!("request from {:?} id {:?}", info.Name(), info.Id());
                let params = WiFiDirectConnectionParameters::new().unwrap();
                params.SetGroupOwnerIntent(15).unwrap();
                params
                    .PreferenceOrderedConfigurationMethods()
                    .unwrap()
                    .Append(WiFiDirectConfigurationMethod::PushButton)
                    .unwrap();
                params
                    .SetPreferredPairingProcedure(WiFiDirectPairingProcedure::GroupOwnerNegotiation)
                    .unwrap();
                let custom: DeviceInformationCustomPairing =
                    info.Pairing().unwrap().Custom().unwrap();
                custom
                    .PairingRequested(&TypedEventHandler::new(
                        |_, args: Ref<DevicePairingRequestedEventArgs>| {
                            if let Ok(args) = args.ok() {
                                println!("  pairing requested: {:?}", args.PairingKind());
                                args.Accept()?;
                            }
                            Ok(())
                        },
                    ))
                    .unwrap();
                let kinds = DevicePairingKinds(
                    DevicePairingKinds::ConfirmOnly.0
                        | DevicePairingKinds::DisplayPin.0
                        | DevicePairingKinds::ProvidePin.0,
                );
                match custom.PairWithProtectionLevelAndSettingsAsync(
                    kinds,
                    DevicePairingProtectionLevel::Default,
                    &params,
                ) {
                    Ok(op) => match op.await {
                        Ok(result) => {
                            println!("  {:?} paired: {:?}", started.elapsed(), result.Status())
                        }
                        Err(e) => println!("  pairing failed: {e}"),
                    },
                    Err(e) => println!("  pairing call failed: {e}"),
                }
                match WiFiDirectDevice::FromIdAsync(&info.Id().unwrap())
                    .unwrap()
                    .await
                {
                    Ok(device) => {
                        println!(
                            "  {:?} connected: {:?}",
                            started.elapsed(),
                            device.ConnectionStatus()
                        );
                        for pair in device.GetConnectionEndpointPairs().unwrap() {
                            println!(
                                "  local {:?} remote {:?}",
                                pair.LocalHostName().and_then(|h| h.CanonicalName()),
                                pair.RemoteHostName().and_then(|h| h.CanonicalName())
                            );
                        }
                        devices.push(device);
                    }
                    Err(e) => println!("  connect failed: {e}"),
                }
            }
            println!("done; {} connected", devices.len());
            let _ = publisher.Stop();
        }

        /// Lists the Wi-Fi Direct devices Windows knows, paired or not,
        /// read-only: `cargo test -p beam-desktop list_wifi_direct_devices -- --ignored --nocapture`.
        #[tokio::test]
        #[ignore = "needs a Wi-Fi Direct capable adapter"]
        async fn list_wifi_direct_devices() {
            let props: Vec<HSTRING> = [
                "System.Devices.Aep.DeviceAddress",
                "System.Devices.Aep.IsPaired",
            ]
            .iter()
            .map(|s| HSTRING::from(*s))
            .collect();
            let op = {
                let iterable = IIterable::<HSTRING>::from(props.clone());
                DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(
                    &HSTRING::from(
                        "System.Devices.Aep.ProtocolId:=\"{0407d24e-53de-4c9a-9ba1-9ced54641188}\"",
                    ),
                    &iterable,
                    DeviceInformationKind::AssociationEndpoint,
                )
                .unwrap()
            };
            for info in op.await.unwrap() {
                let get = |k: &HSTRING| {
                    info.Properties()
                        .and_then(|p| p.Lookup(k))
                        .map(|v| format!("{v:?}"))
                        .unwrap_or_default()
                };
                println!(
                    "{:?} id={:?} addr={} paired={} pairing.IsPaired={:?}",
                    info.Name(),
                    info.Id(),
                    get(&props[0]),
                    get(&props[1]),
                    info.Pairing().and_then(|p| p.IsPaired())
                );
            }
        }

        /// Makes Windows forget Wi-Fi Direct pairings: the one with
        /// BEAM_FORGET_ADDRESS, or all of them:
        /// `cargo test -p beam-desktop forget_wifi_direct_pairings -- --ignored --nocapture`.
        #[tokio::test]
        #[ignore = "changes this PC's Wi-Fi Direct pairings"]
        async fn forget_wifi_direct_pairings() {
            let address = std::env::var("BEAM_FORGET_ADDRESS")
                .unwrap_or_default()
                .to_lowercase();
            let n = forget_direct_pairing(&address).await;
            println!("forgot {n} pairing(s)");
        }

        /// Phase 8 feasibility: host a Wi-Fi Direct group with a name and
        /// passphrase (what an Android phone can join without a prompt) and
        /// check this laptop stays on its Wi-Fi network meanwhile:
        /// `cargo test -p beam-desktop wifi_direct_group_beside_the_wifi -- --ignored --nocapture`.
        #[tokio::test]
        #[ignore = "needs a Wi-Fi Direct capable adapter"]
        async fn wifi_direct_group_beside_the_wifi() {
            use windows::Devices::WiFiDirect::{
                WiFiDirectAdvertisementPublisher,
                WiFiDirectAdvertisementPublisherStatusChangedEventArgs,
            };
            let publisher = WiFiDirectAdvertisementPublisher::new().unwrap();
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            publisher
                .StatusChanged(&TypedEventHandler::new(
                    move |_, args: Ref<WiFiDirectAdvertisementPublisherStatusChangedEventArgs>| {
                        if let Ok(args) = args.ok() {
                            let _ =
                                tx.send(format!("{:?} error {:?}", args.Status()?, args.Error()?));
                        }
                        Ok(())
                    },
                ))
                .unwrap();
            let advertisement = publisher.Advertisement().unwrap();
            advertisement
                .SetIsAutonomousGroupOwnerEnabled(true)
                .unwrap();
            let legacy = advertisement.LegacySettings().unwrap();
            legacy.SetIsEnabled(true).unwrap();
            legacy.SetSsid(&HSTRING::from("DIRECT-Bm-spike")).unwrap();
            let credential = PasswordCredential::new().unwrap();
            credential
                .SetPassword(&HSTRING::from("beam-spike-12345"))
                .unwrap();
            legacy.SetPassphrase(&credential).unwrap();

            let before = lan_ips();
            println!("addresses before: {before:?}");
            publisher.Start().unwrap();
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(20) {
                if let Ok(Some(status)) =
                    tokio::time::timeout(Duration::from_secs(1), rx.recv()).await
                {
                    println!("{:?} publisher: {status}", started.elapsed());
                }
                let now = lan_ips();
                if now != before {
                    println!("{:?} addresses now: {now:?}", started.elapsed());
                    break;
                }
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
            println!("status after 3 s more: {:?}", publisher.Status());
            println!("addresses: {:?}", lan_ips());
            let wlan = std::process::Command::new("netsh")
                .args(["wlan", "show", "interfaces"])
                .output()
                .unwrap();
            for line in String::from_utf8_lossy(&wlan.stdout).lines() {
                if line.contains("State") || line.contains(" SSID") || line.contains("Channel") {
                    println!("  {}", line.trim());
                }
            }
            publisher.Stop().unwrap();
        }

        #[tokio::test]
        #[ignore = "needs a Bluetooth LE adapter"]
        async fn the_scanner_hears_advertisements() {
            let watcher = BluetoothLEAdvertisementWatcher::new().unwrap();
            watcher
                .SetScanningMode(BluetoothLEScanningMode::Active)
                .unwrap();
            let heard = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let count = heard.clone();
            watcher
                .Received(&TypedEventHandler::new(
                    move |_, _: Ref<BluetoothLEAdvertisementReceivedEventArgs>| {
                        count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        Ok(())
                    },
                ))
                .unwrap();
            watcher.Start().unwrap();
            tokio::time::sleep(Duration::from_secs(3)).await;
            watcher.Stop().unwrap();
            println!(
                "heard {} advertisements in 3 s",
                heard.load(std::sync::atomic::Ordering::Relaxed)
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_decline_explains_itself() {
        let response = ConnectResponse {
            link: None,
            addr: None,
            hotspot: None,
            reason: Some("busy".into()),
        };
        assert_eq!(declined(&response, "Pixel"), "busy");
        let silent = ConnectResponse {
            reason: None,
            ..response
        };
        assert_eq!(declined(&silent, "Pixel"), "Pixel declined");
    }
}
