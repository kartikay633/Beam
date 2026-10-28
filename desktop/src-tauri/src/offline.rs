//! Beam with no shared network: Bluetooth LE to find phones and agree on a
//! link, then the phone's hotspot for the transfer (SPEC.md 4.2 and 5).
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
//!   to it, take its hotspot, and say hello with our port;
//! - **joining the phone's hotspot** (`WiFiAdapter`), then going back to the
//!   previous Wi-Fi network when the transfer is over.
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

/// A phone heard over Bluetooth.
#[derive(Debug, Clone)]
pub struct BleSighting {
    pub device: BleDevice,
    pub address: u64,
    pub random_address: bool,
    pub rssi: i16,
    pub seen: Instant,
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

/// Where to send after a Bluetooth handshake. Dropping it gives back any
/// hotspot this laptop joined for the transfer.
pub struct OfflineLink {
    pub addr: SocketAddr,
    pub expect: Option<DeviceId>,
    _wifi: Option<WifiLease>,
}

#[cfg(windows)]
pub use win::{Radio, WifiLease};

#[cfg(not(windows))]
pub use stub::{Radio, WifiLease};

#[cfg(not(windows))]
mod stub {
    use super::*;

    pub struct WifiLease;

    pub struct Radio;

    impl Radio {
        pub async fn start(_state: Weak<AppState>) -> Result<Arc<Radio>, String> {
            Err("Bluetooth is only supported on Windows".into())
        }
        pub fn update_advertisement(&self) {}
        pub async fn connect(
            &self,
            _sighting: &BleSighting,
            _status: &(dyn Fn(&str) + Sync),
        ) -> Result<OfflineLink, String> {
            Err("Bluetooth is only supported on Windows".into())
        }
    }
}

/// Where to connect after joining a phone's hotspot: the address it gave,
/// or (when it couldn't tell) its gateway address on our new subnet.
fn hotspot_target(addr: Option<&str>, our_ip: IpAddr) -> Result<SocketAddr, String> {
    let addr: SocketAddr = addr
        .ok_or("the phone didn't say where to connect")?
        .parse()
        .map_err(|_| "the phone sent an unreadable address".to_string())?;
    if !addr.ip().is_unspecified() {
        return Ok(addr);
    }
    match our_ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            Ok(SocketAddr::new(IpAddr::from([a, b, c, 1]), addr.port()))
        }
        IpAddr::V6(_) => Err("no IPv4 address on the phone's hotspot".into()),
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

    use beam_core::ble::{
        self, fragment, BleAdvert, ConnectRequest, GattClientHandshake, LinkKind, Reassembler,
        PREFERRED_MTU,
    };
    use windows::core::{Ref, GUID, HSTRING};
    use windows::Devices::Bluetooth::Advertisement::{
        BluetoothLEAdvertisementDataSection, BluetoothLEAdvertisementPublisher,
        BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher,
        BluetoothLEScanningMode,
    };
    use windows::Devices::Bluetooth::GenericAttributeProfile::{
        GattCharacteristic, GattClientCharacteristicConfigurationDescriptorValue,
        GattCommunicationStatus, GattSession, GattValueChangedEventArgs, GattWriteOption,
    };
    use windows::Devices::Bluetooth::{
        BluetoothAdapter, BluetoothAddressType, BluetoothCacheMode, BluetoothLEDevice,
    };
    use windows::Devices::WiFi::{
        WiFiAccessStatus, WiFiAdapter, WiFiAvailableNetwork, WiFiConnectionStatus,
        WiFiReconnectionKind,
    };
    use windows::Foundation::TypedEventHandler;
    use windows::Networking::Connectivity::NetworkInformation;
    use windows::Networking::HostNameType;
    use windows::Security::Credentials::PasswordCredential;
    use windows::Storage::Streams::{DataReader, DataWriter, IBuffer};

    use crate::state::lock;

    type WinResult<T> = windows::core::Result<T>;

    /// A call from the same phone within this long is the same call.
    const CALL_DEBOUNCE: Duration = Duration::from_secs(30);

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
        calls: Mutex<HashMap<u64, Instant>>,
        /// Hotspots joined to receive from a phone, released when done.
        receiving_wifi: Mutex<Vec<WifiLease>>,
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
                receiving_wifi: Mutex::default(),
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
            radio.clone().release_hotspots_when_idle();
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

        /// A phone called: connect to it, take its hotspot (or LAN), and
        /// tell it where we receive.
        fn answer_call(self: Arc<Self>, address: u64, random: bool) {
            {
                let mut calls = lock(&self.calls);
                calls.retain(|_, since| since.elapsed() < CALL_DEBOUNCE);
                if calls.contains_key(&address) {
                    return;
                }
                calls.insert(address, Instant::now());
            }
            tauri::async_runtime::spawn(async move {
                if let Err(problem) = self.take_call(address, random).await {
                    if let Some(state) = self.state.upgrade() {
                        state
                            .show_error(format!("A phone tried to send over Bluetooth: {problem}"));
                    }
                }
            });
        }

        async fn take_call(&self, address: u64, random: bool) -> Result<(), String> {
            let state = self.state.upgrade().ok_or("Beam is closing")?;
            let device = Self::device(address, random).await?;
            let request = ConnectRequest {
                caps: 0,
                links: vec![LinkKind::Lan, LinkKind::Hotspot],
                lan: lan_ips().iter().map(ToString::to_string).collect(),
                hotspot: None,
                port: Some(state.port()),
            };
            let response = Self::handshake(&device, &request).await.map_err(text);
            let _ = device.Close();
            let response = response??;
            match response.link {
                // On one network: the phone sends to our LAN address.
                Some(LinkKind::Lan) => Ok(()),
                Some(LinkKind::Hotspot) => {
                    let hotspot = response
                        .hotspot
                        .as_ref()
                        .ok_or("the phone offered no hotspot")?;
                    let lease = WifiLease::join(&hotspot.ssid, &hotspot.passphrase).await?;
                    let phone = hotspot_target(response.addr.as_deref(), lease.ip)?;
                    lock(&self.receiving_wifi).push(lease);
                    self.say_hello(&state, phone).await
                }
                _ => Err(declined(&response, "the phone")),
            }
        }

        /// Tells the phone our address on its hotspot until its transfer
        /// arrives (UDP can be lost; the phone acts on the first).
        async fn say_hello(&self, state: &AppState, phone: SocketAddr) -> Result<(), String> {
            let socket = tokio::net::UdpSocket::bind("0.0.0.0:0")
                .await
                .map_err(|e| e.to_string())?;
            let hello = ble::hello_packet(&state.device, state.port());
            for _ in 0..40 {
                let _ = socket.send_to(&hello, phone).await;
                if state.is_receiving() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Ok(())
        }

        /// Gives back hotspots joined to receive, once nothing is arriving.
        fn release_hotspots_when_idle(self: Arc<Self>) {
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    let Some(state) = self.state.upgrade() else {
                        return;
                    };
                    if !state.is_receiving() {
                        // Leave time for the transfer to start before letting go.
                        lock(&self.receiving_wifi)
                            .retain(|lease| lease.joined.elapsed() < Duration::from_secs(90));
                    }
                }
            });
        }

        // ---------------------------------------------------- sending

        /// Asks a phone heard over Bluetooth for a link, joining its hotspot
        /// when it offers one.
        pub async fn connect(
            &self,
            sighting: &BleSighting,
            status: &(dyn Fn(&str) + Sync),
        ) -> Result<OfflineLink, String> {
            let name = &sighting.device.name;
            status("Connecting over Bluetooth…");
            let device = Self::device(sighting.address, sighting.random_address).await?;
            let request = ConnectRequest {
                caps: 0,
                links: vec![LinkKind::Lan, LinkKind::Hotspot],
                lan: lan_ips().iter().map(ToString::to_string).collect(),
                hotspot: None,
                port: None,
            };
            status(&format!("Waiting for {name} to start its Wi-Fi…"));
            let response = Self::handshake(&device, &request).await.map_err(text);
            let _ = device.Close();
            let response = response??;
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
                    _wifi: None,
                }),
                Some(LinkKind::Hotspot) => {
                    let hotspot = response
                        .hotspot
                        .as_ref()
                        .ok_or("the phone offered no hotspot")?;
                    status(&format!("Joining {name}'s Wi-Fi…"));
                    let lease = WifiLease::join(&hotspot.ssid, &hotspot.passphrase).await?;
                    Ok(OfflineLink {
                        addr: hotspot_target(response.addr.as_deref(), lease.ip)?,
                        expect,
                        _wifi: Some(lease),
                    })
                }
                Some(LinkKind::WifiDirect) => Err("Wi-Fi Direct isn't supported yet".into()),
                None => Err(declined(&response, name)),
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
            let services = device
                .GetGattServicesForUuidWithCacheModeAsync(
                    guid(ble::SERVICE_UUID),
                    BluetoothCacheMode::Uncached,
                )?
                .await?;
            if services.Status()? != GattCommunicationStatus::Success
                || services.Services()?.Size()? == 0
            {
                return Ok(Err("that phone isn't running Beam".into()));
            }
            let service = services.Services()?.GetAt(0)?;
            let mut found = Vec::new();
            for uuid in [
                ble::INFO_UUID,
                ble::CONNECT_REQUEST_UUID,
                ble::CONNECT_RESPONSE_UUID,
            ] {
                let result = service
                    .GetCharacteristicsForUuidWithCacheModeAsync(
                        guid(uuid),
                        BluetoothCacheMode::Uncached,
                    )?
                    .await?;
                if result.Characteristics()?.Size()? == 0 {
                    return Ok(Err("the phone's Beam service is incomplete".into()));
                }
                found.push(result.Characteristics()?.GetAt(0)?);
            }
            let [info_char, request_char, response_char]: [GattCharacteristic; 3] = found
                .try_into()
                .map_err(|_| windows::core::Error::empty())?;
            let session = GattSession::FromDeviceIdAsync(&device.BluetoothDeviceId()?)?.await?;
            let _ = session.SetMaintainConnection(true);
            let mtu = (session.MaxPduSize().unwrap_or(23) as usize).min(PREFERRED_MTU);

            let read = info_char
                .ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached)?
                .await?;
            if read.Status()? != GattCommunicationStatus::Success {
                return Ok(Err("couldn't read the phone's Beam info".into()));
            }
            let info = from_buffer(&read.Value()?)?;

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
            // The phone may start its hotspot before answering.
            let mut reassembler = Reassembler::default();
            let whole = tokio::time::timeout(Duration::from_secs(45), async {
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
            Ok(opener.open(&whole).map_err(|e| e.to_string()))
        }
    }

    // -------------------------------------------------------- Wi-Fi

    /// A Wi-Fi network joined for a transfer. Dropping it leaves the network
    /// and reconnects to the one we were on before.
    pub struct WifiLease {
        adapter: WiFiAdapter,
        previous: Option<String>,
        pub ip: IpAddr,
        joined: Instant,
    }

    impl WifiLease {
        pub async fn join(ssid: &str, passphrase: &str) -> Result<WifiLease, String> {
            let access = WiFiAdapter::RequestAccessAsync()
                .map_err(text)?
                .await
                .map_err(text)?;
            if access != WiFiAccessStatus::Allowed {
                return Err(
                    "Windows didn't let Beam use Wi-Fi. Check Settings → Privacy → Location."
                        .into(),
                );
            }
            let find = WiFiAdapter::FindAllAdaptersAsync().map_err(text)?;
            let adapter = {
                // Not Send: don't hold it across an await.
                let adapters = find.await.map_err(text)?;
                adapters
                    .GetAt(0)
                    .map_err(|_| "this computer has no Wi-Fi".to_string())?
            };
            let previous = Self::connected_ssid(&adapter).await;
            let network = Self::find(&adapter, ssid)
                .await?
                .ok_or_else(|| format!("couldn't find the phone's Wi-Fi \"{ssid}\""))?;
            let credential = PasswordCredential::new().map_err(text)?;
            credential
                .SetPassword(&HSTRING::from(passphrase))
                .map_err(text)?;
            let connect = adapter
                .ConnectWithPasswordCredentialAsync(
                    &network,
                    WiFiReconnectionKind::Manual,
                    &credential,
                )
                .map_err(text)?;
            let result = connect.await.map_err(text)?;
            if result.ConnectionStatus().map_err(text)? != WiFiConnectionStatus::Success {
                return Err(format!("couldn't join \"{ssid}\""));
            }
            let id = adapter
                .NetworkAdapter()
                .and_then(|n| n.NetworkAdapterId())
                .map_err(text)?;
            let mut lease = WifiLease {
                adapter,
                previous,
                ip: IpAddr::from([0, 0, 0, 0]),
                joined: Instant::now(),
            };
            // Wait for an address from the phone's DHCP.
            for _ in 0..30 {
                if let Some(ip) = Self::ipv4_on(id) {
                    lease.ip = ip;
                    return Ok(lease);
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            // Dropping the lease reconnects the previous network.
            Err("joined the phone's Wi-Fi but got no address".into())
        }

        async fn connected_ssid(adapter: &WiFiAdapter) -> Option<String> {
            let profile = adapter
                .NetworkAdapter()
                .ok()?
                .GetConnectedProfileAsync()
                .ok()?
                .await
                .ok()?;
            let ssid = profile
                .WlanConnectionProfileDetails()
                .ok()?
                .GetConnectedSsid()
                .ok()?
                .to_string();
            (!ssid.is_empty()).then_some(ssid)
        }

        async fn find(
            adapter: &WiFiAdapter,
            ssid: &str,
        ) -> Result<Option<WiFiAvailableNetwork>, String> {
            // A new hotspot can take a few scans to show up.
            for _ in 0..8 {
                adapter.ScanAsync().map_err(text)?.await.map_err(text)?;
                let found = {
                    let networks = adapter
                        .NetworkReport()
                        .and_then(|r| r.AvailableNetworks())
                        .map_err(text)?;
                    networks
                        .into_iter()
                        .find(|n| n.Ssid().map(|s| s == ssid).unwrap_or(false))
                };
                if found.is_some() {
                    return Ok(found);
                }
                tokio::time::sleep(Duration::from_millis(700)).await;
            }
            Ok(None)
        }

        fn ipv4_on(adapter_id: GUID) -> Option<IpAddr> {
            NetworkInformation::GetHostNames()
                .ok()?
                .into_iter()
                .find_map(|host| {
                    if host.Type().ok()? != HostNameType::Ipv4 {
                        return None;
                    }
                    let id = host
                        .IPInformation()
                        .ok()?
                        .NetworkAdapter()
                        .ok()?
                        .NetworkAdapterId()
                        .ok()?;
                    (id == adapter_id)
                        .then(|| host.CanonicalName().ok()?.to_string().parse().ok())?
                })
        }
    }

    impl Drop for WifiLease {
        fn drop(&mut self) {
            let adapter = self.adapter.clone();
            let previous = self.previous.take();
            tauri::async_runtime::spawn(async move {
                let _ = adapter.Disconnect();
                // Windows usually reconnects by itself; ask, in case it doesn't.
                if let Some(ssid) = previous {
                    if let Ok(Some(network)) = WifiLease::find(&adapter, &ssid).await {
                        if let Ok(connect) =
                            adapter.ConnectAsync(&network, WiFiReconnectionKind::Automatic)
                        {
                            let _ = connect.await;
                        }
                    }
                }
            });
        }
    }

    /// Run on a machine with Bluetooth LE: `cargo test -p beam-desktop -- --ignored`.
    #[cfg(test)]
    mod hardware {
        use super::*;

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
    fn the_hotspot_gateway_stands_in_for_an_unknown_phone_address() {
        let ours: IpAddr = "192.168.49.23".parse().unwrap();
        assert_eq!(
            hotspot_target(Some("0.0.0.0:41234"), ours).unwrap(),
            "192.168.49.1:41234".parse().unwrap()
        );
        assert_eq!(
            hotspot_target(Some("192.168.49.7:41234"), ours).unwrap(),
            "192.168.49.7:41234".parse().unwrap()
        );
        assert!(hotspot_target(None, ours).is_err());
    }

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
