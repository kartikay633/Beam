//! Bluetooth LE for the apps: what to advertise, what a scan result means,
//! and the encrypted GATT handshake that agrees on a Wi-Fi link
//! (`beam_core::ble`, SPEC.md 4.2). The radios are the app's; everything
//! both sides must agree on byte for byte happens here.

use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{failed, lock, BeamError, DeviceKind, Inner, NearbyDevice, NearbyLink};
use beam_core::ble::{
    self, caps, ConnectRequest, ConnectResponse, GattClientHandshake, GattServerHandshake,
    HotspotInfo, LinkKind, Reassembler, ResponseOpener, ResponseSealer,
};

/// A device not heard from for this long is gone.
pub(crate) const BLE_EXPIRY: Duration = Duration::from_secs(12);

/// What this device can do over Wi-Fi, for advertisements and handshakes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct BleCapabilities {
    /// Can host a hotspot for the other device to join (Android).
    pub hotspot: bool,
    pub wifi_direct: bool,
}

impl BleCapabilities {
    pub(crate) fn bits(self, on_lan: bool) -> u8 {
        let mut bits = 0;
        if self.hotspot {
            bits |= caps::HOTSPOT;
        }
        if self.wifi_direct {
            bits |= caps::WIFI_DIRECT;
        }
        if on_lan {
            bits |= caps::LAN;
        }
        bits
    }
}

/// Service data and scan-response name to advertise under
/// `beam_core::ble::SERVICE_UUID`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BleAdvertisement {
    pub service_uuid: String,
    /// 10 bytes.
    pub service_data: Vec<u8>,
    /// For the scan response; `None` in trusted-only mode.
    pub local_name: Option<String>,
}

/// The UUIDs of the Beam GATT service and its characteristics.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BleUuids {
    pub service: String,
    pub info: String,
    pub connect_request: String,
    pub connect_response: String,
    /// The MTU to request after connecting.
    pub preferred_mtu: u32,
}

#[uniffi::export]
pub fn ble_uuids() -> BleUuids {
    BleUuids {
        service: ble::SERVICE_UUID.into(),
        info: ble::INFO_UUID.into(),
        connect_request: ble::CONNECT_REQUEST_UUID.into(),
        connect_response: ble::CONNECT_RESPONSE_UUID.into(),
        preferred_mtu: ble::PREFERRED_MTU as u32,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum BleLink {
    Lan,
    Hotspot,
    WifiDirect,
}

impl From<BleLink> for LinkKind {
    fn from(link: BleLink) -> Self {
        match link {
            BleLink::Lan => LinkKind::Lan,
            BleLink::Hotspot => LinkKind::Hotspot,
            BleLink::WifiDirect => LinkKind::WifiDirect,
        }
    }
}

impl From<LinkKind> for BleLink {
    fn from(link: LinkKind) -> Self {
        match link {
            LinkKind::Lan => BleLink::Lan,
            LinkKind::Hotspot => BleLink::Hotspot,
            LinkKind::WifiDirect => BleLink::WifiDirect,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HotspotCredentials {
    pub ssid: String,
    pub passphrase: String,
}

impl From<HotspotCredentials> for HotspotInfo {
    fn from(h: HotspotCredentials) -> Self {
        HotspotInfo {
            ssid: h.ssid,
            passphrase: h.passphrase,
        }
    }
}

impl From<HotspotInfo> for HotspotCredentials {
    fn from(h: HotspotInfo) -> Self {
        HotspotCredentials {
            ssid: h.ssid,
            passphrase: h.passphrase,
        }
    }
}

/// The sender's `CONNECT_REQUEST`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BleConnectRequest {
    /// Links the sender can use, best first.
    pub links: Vec<BleLink>,
    /// The sender's LAN IP addresses.
    pub lan: Vec<String>,
    /// Set when the sender hosts a hotspot for this transfer.
    pub hotspot: Option<HotspotCredentials>,
    /// The sender's capability bits (`beam_core::ble::caps`).
    pub capabilities: u8,
}

/// The receiver's `CONNECT_RESPONSE`: the link it chose.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BleConnectResponse {
    /// `None` when the receiver declines; `reason` says why.
    pub link: Option<BleLink>,
    /// `ip:port` to connect to.
    pub address: Option<String>,
    /// Set when the receiver hosts a hotspot for the sender to join.
    pub hotspot: Option<HotspotCredentials>,
    pub reason: Option<String>,
}

impl From<ConnectResponse> for BleConnectResponse {
    fn from(r: ConnectResponse) -> Self {
        BleConnectResponse {
            link: r.link.map(Into::into),
            address: r.addr,
            hotspot: r.hotspot.map(Into::into),
            reason: r.reason,
        }
    }
}

impl From<BleConnectResponse> for ConnectResponse {
    fn from(r: BleConnectResponse) -> Self {
        ConnectResponse {
            link: r.link.map(Into::into),
            addr: r.address,
            hotspot: r.hotspot.map(Into::into),
            reason: r.reason,
        }
    }
}

/// What `INFO` said about the receiver.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BleDeviceInfo {
    /// Left out by devices in trusted-only mode.
    pub name: Option<String>,
    pub kind: DeviceKind,
    pub device_id: Option<String>,
    pub capabilities: u8,
}

fn ip_list(addresses: &[String]) -> Vec<IpAddr> {
    addresses
        .iter()
        .filter_map(|a| {
            a.parse::<IpAddr>()
                .ok()
                .or_else(|| a.parse::<std::net::SocketAddr>().ok().map(|s| s.ip()))
        })
        .collect()
}

/// Our address on a network the sender is also on (IPv4 /24), if any.
#[uniffi::export]
pub fn ble_shared_lan(theirs: Vec<String>, ours: Vec<String>) -> Option<String> {
    ble::shared_lan(&ip_list(&theirs), &ip_list(&ours)).map(|ip| ip.to_string())
}

/// The receiver's side of one GATT connection. Serve [`info`](Self::info)
/// on `INFO` reads, feed each `CONNECT_REQUEST` write to
/// [`receive`](Self::receive), and notify the fragments from
/// [`respond`](Self::respond).
#[derive(uniffi::Object)]
pub struct BleServerSession {
    info: Vec<u8>,
    handshake: Mutex<Option<GattServerHandshake>>,
    reassembler: Mutex<Reassembler>,
    sealer: Mutex<Option<ResponseSealer>>,
}

#[uniffi::export]
impl BleServerSession {
    /// The `INFO` value for this connection.
    pub fn info(&self) -> Vec<u8> {
        self.info.clone()
    }

    /// One `CONNECT_REQUEST` write (a fragment). Returns the request once
    /// every fragment is in.
    pub fn receive(&self, fragment: Vec<u8>) -> Result<Option<BleConnectRequest>, BeamError> {
        let Some(whole) = lock(&self.reassembler).push(&fragment).map_err(failed)? else {
            return Ok(None);
        };
        let handshake = lock(&self.handshake)
            .take()
            .ok_or_else(|| failed("this connection already sent its request"))?;
        let (request, sealer) = handshake.accept_request(&whole).map_err(failed)?;
        *lock(&self.sealer) = Some(sealer);
        Ok(Some(BleConnectRequest {
            links: request.links.into_iter().map(Into::into).collect(),
            lan: request.lan,
            hotspot: request.hotspot.map(Into::into),
            capabilities: request.caps,
        }))
    }

    /// Seals the response, split for notifications at `mtu`.
    pub fn respond(
        &self,
        response: BleConnectResponse,
        mtu: u32,
    ) -> Result<Vec<Vec<u8>>, BeamError> {
        let sealer = lock(&self.sealer)
            .take()
            .ok_or_else(|| failed("no request to respond to"))?;
        ble::fragment(&sealer.seal(&response.into()), mtu as usize).map_err(failed)
    }
}

/// The sender's side: made from the receiver's `INFO`.
#[derive(uniffi::Object)]
pub struct BleClientSession {
    info: BleDeviceInfo,
    request: Vec<u8>,
    opener: Mutex<Option<ResponseOpener>>,
    reassembler: Mutex<Reassembler>,
}

#[uniffi::export]
impl BleClientSession {
    pub fn device(&self) -> BleDeviceInfo {
        self.info.clone()
    }

    /// The sealed `CONNECT_REQUEST`, split into writes for `mtu`.
    pub fn request_fragments(&self, mtu: u32) -> Result<Vec<Vec<u8>>, BeamError> {
        ble::fragment(&self.request, mtu as usize).map_err(failed)
    }

    /// One `CONNECT_RESPONSE` notification. Returns the response once every
    /// fragment is in.
    pub fn receive(&self, fragment: Vec<u8>) -> Result<Option<BleConnectResponse>, BeamError> {
        let Some(whole) = lock(&self.reassembler).push(&fragment).map_err(failed)? else {
            return Ok(None);
        };
        let opener = lock(&self.opener)
            .take()
            .ok_or_else(|| failed("this response was already read"))?;
        Ok(Some(opener.open(&whole).map_err(failed)?.into()))
    }
}

/// A device heard over Bluetooth, keyed by [`ble_key`].
pub(crate) struct BleEntry {
    pub device: NearbyDevice,
    pub seen: Instant,
    /// Whether the app has been told about it (a LAN sighting of the same
    /// device takes precedence).
    pub shown: bool,
}

pub(crate) fn ble_key(seen: &ble::BleDevice) -> String {
    match seen.device_id {
        Some(id) => id.to_string(),
        None => format!("ble-{}", hex::encode(seen.id)),
    }
}

impl Inner {
    pub(crate) fn ble_advertisement(&self) -> Option<BleAdvertisement> {
        let capabilities = self.ble?;
        let (advert, name) = ble::advertisement(
            &self.device,
            self.visibility.mode(),
            capabilities.bits(!lock(&self.lan_addresses).is_empty()),
        )?;
        Some(BleAdvertisement {
            service_uuid: ble::SERVICE_UUID.into(),
            service_data: advert.encode().to_vec(),
            local_name: name,
        })
    }

    /// Tells the app what to advertise whenever it changes.
    pub(crate) async fn advertise_ble(self: Arc<Self>) {
        let mut changes = self.visibility.subscribe();
        let mut current = None;
        let mut first = true;
        loop {
            let wanted = self.ble_advertisement();
            if first || wanted != current {
                self.listener.ble_advertisement_changed(wanted.clone());
                current = wanted;
                first = false;
            }
            tokio::select! {
                _ = changes.changed() => {}
                _ = tokio::time::sleep_until(beam_core::discovery::next_announcement_change(&self.visibility)) => {}
                // The LAN flag can change without visibility changing.
                _ = tokio::time::sleep(Duration::from_secs(60)) => {}
            }
        }
    }

    pub(crate) fn ble_seen(
        &self,
        address: String,
        service_data: &[u8],
        local_name: Option<String>,
        rssi: i16,
    ) {
        let Some(advert) = ble::BleAdvert::decode(service_data) else {
            return;
        };
        let trusted = self.trust.list().unwrap_or_default();
        let Some(seen) =
            ble::interpret_advert(&advert, local_name.as_deref(), &self.device, &trusted)
        else {
            return;
        };
        let key = ble_key(&seen);
        let device = NearbyDevice {
            device_id: key.clone(),
            name: seen.name,
            kind: Some(seen.device_type).into(),
            address: None,
            trusted: seen.trusted.is_some(),
            link: NearbyLink::Bluetooth,
            ble_address: Some(address),
            rssi: Some(rssi),
        };
        let on_lan = seen
            .device_id
            .is_some_and(|id| lock(&self.nearby).contains_key(&id));
        let mut entries = lock(&self.ble_devices);
        let previous = entries.get(&key);
        // Report new devices, and changes other than signal strength jitter.
        let changed = previous.is_none_or(|p| {
            !p.shown
                || p.device.name != device.name
                || p.device.ble_address != device.ble_address
                || p.device
                    .rssi
                    .zip(device.rssi)
                    .is_none_or(|(a, b)| (a - b).abs() >= 6)
        });
        let show = !on_lan && changed;
        let shown = !on_lan && (show || previous.is_some_and(|p| p.shown));
        entries.insert(
            key,
            BleEntry {
                device: device.clone(),
                seen: Instant::now(),
                shown,
            },
        );
        drop(entries);
        if show {
            self.listener.device_found(device);
        }
    }

    /// Forgets Bluetooth devices that went quiet.
    pub(crate) async fn expire_ble(self: Arc<Self>) {
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        loop {
            tick.tick().await;
            let gone: Vec<(String, bool)> = {
                let mut entries = lock(&self.ble_devices);
                let stale: Vec<String> = entries
                    .iter()
                    .filter(|(_, e)| e.seen.elapsed() > BLE_EXPIRY)
                    .map(|(k, _)| k.clone())
                    .collect();
                stale
                    .into_iter()
                    .filter_map(|k| entries.remove(&k).map(|e| (k, e.shown)))
                    .collect()
            };
            for (key, shown) in gone {
                if shown {
                    self.listener.device_lost(key);
                }
            }
        }
    }

    /// A LAN sighting replaced the Bluetooth one, or ended: keep the
    /// Bluetooth entry in step.
    pub(crate) fn lan_changed(&self, device_id: &str, on_lan: bool) {
        let mut entries = lock(&self.ble_devices);
        let Some(entry) = entries.get_mut(device_id) else {
            return;
        };
        entry.shown = !on_lan;
        let device = entry.device.clone();
        drop(entries);
        if !on_lan {
            self.listener.device_found(device);
        }
    }

    pub(crate) fn ble_server_session(&self) -> BleServerSession {
        let handshake = GattServerHandshake::new();
        let capabilities = self.ble.unwrap_or(BleCapabilities {
            hotspot: false,
            wifi_direct: false,
        });
        let info = handshake.info(
            &self.device,
            self.visibility.mode(),
            capabilities.bits(!lock(&self.lan_addresses).is_empty()),
        );
        BleServerSession {
            info,
            handshake: Mutex::new(Some(handshake)),
            reassembler: Mutex::default(),
            sealer: Mutex::default(),
        }
    }

    pub(crate) fn ble_client_session(
        &self,
        info: &[u8],
        request: BleConnectRequest,
    ) -> Result<BleClientSession, BeamError> {
        let request = ConnectRequest {
            caps: request.capabilities,
            links: request.links.into_iter().map(Into::into).collect(),
            lan: request.lan,
            hotspot: request.hotspot.map(Into::into),
        };
        let (info, data, opener) = GattClientHandshake::request(info, &request).map_err(failed)?;
        Ok(BleClientSession {
            info: BleDeviceInfo {
                name: info.name,
                kind: Some(info.device_type).into(),
                device_id: info.fingerprint.map(|id| id.to_string()),
                capabilities: info.caps,
            },
            request: data,
            opener: Mutex::new(Some(opener)),
            reassembler: Mutex::default(),
        })
    }
}
