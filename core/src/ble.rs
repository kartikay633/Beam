//! Finding devices with Bluetooth LE, and agreeing on a link, when there is
//! no shared network (SPEC.md 4.2).
//!
//! The radios themselves are platform code (Android's `BluetoothLeAdvertiser`
//! and GATT server, WinRT on Windows). This module is everything both
//! platforms must agree on, byte for byte:
//!
//! - the 10-byte **advertisement** in the service data, and how a scanner
//!   turns it into a device to show ([`advertisement`], [`interpret_advert`]);
//! - the **GATT handshake**: the receiver's `INFO` (with a fresh ephemeral
//!   X25519 key), the sender's encrypted `CONNECT_REQUEST`, and the
//!   receiver's encrypted `CONNECT_RESPONSE` naming the link to use
//!   ([`GattServerHandshake`], [`GattClientHandshake`]);
//! - **fragmentation** of payloads larger than one ATT write
//!   ([`fragment`], [`Reassembler`]).
//!
//! The GATT payloads are encrypted because they can carry a Wi-Fi password:
//! X25519 between the two ephemeral keys → HKDF-SHA256 → ChaCha20-Poly1305,
//! one key per direction. This is *not* authentication: anyone can read
//! `INFO` and answer it. Who is who is proven afterwards by the Noise
//! handshake over the link, as on a LAN.

use std::collections::HashMap;
use std::net::IpAddr;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::discovery::{current_window, rotating_id, truncate};
use crate::messages::{DeviceId, DeviceType};
use crate::transfer::LocalDevice;
use crate::trust::TrustedDevice;
use crate::visibility::VisibilityMode;

/// The Beam GATT service, also the service-data UUID in advertisements.
pub const SERVICE_UUID: &str = "af59893f-ddbd-4717-96b0-c74e6ab25e2b";
/// `INFO` (read): [`DeviceInfo`] as CBOR.
pub const INFO_UUID: &str = "af59893f-ddbd-4717-96b0-c74e6ab25e2c";
/// `CONNECT_REQUEST` (write): sender's ephemeral key ‖ sealed [`ConnectRequest`].
pub const CONNECT_REQUEST_UUID: &str = "af59893f-ddbd-4717-96b0-c74e6ab25e2d";
/// `CONNECT_RESPONSE` (notify): sealed [`ConnectResponse`].
pub const CONNECT_RESPONSE_UUID: &str = "af59893f-ddbd-4717-96b0-c74e6ab25e2e";

/// Version of the advertisement and GATT formats.
pub const BLE_VERSION: u8 = 1;
/// Service data length (SPEC.md 4.2).
pub const ADVERT_LEN: usize = 10;
/// A legacy scan response holds 31 bytes; 2 go to the AD header.
pub const MAX_SCAN_NAME_BYTES: usize = 29;
/// The largest reassembled GATT payload accepted.
pub const MAX_PAYLOAD: usize = 4096;
/// The MTU to ask for after connecting.
pub const PREFERRED_MTU: usize = 517;

/// Bits of the advertisement's flags byte, and of `caps` in the GATT messages.
pub mod caps {
    /// Accepting transfers (visibility isn't off).
    pub const ACCEPTING: u8 = 1 << 0;
    /// On a network, with a LAN address to offer.
    pub const LAN: u8 = 1 << 1;
    /// Can do Wi-Fi Direct.
    pub const WIFI_DIRECT: u8 = 1 << 2;
    /// Can host a hotspot (Android's local-only hotspot).
    pub const HOTSPOT: u8 = 1 << 3;
    /// Advertisement only: the id is the trusted-only rotating id, not a
    /// device id prefix.
    pub const ROTATING_ID: u8 = 1 << 4;
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BleError {
    #[error("malformed {0}")]
    Malformed(&'static str),
    #[error("unsupported Beam BLE version {0}")]
    Version(u8),
    #[error("the payload could not be decrypted")]
    Decrypt,
    #[error("the peer's key is not usable")]
    WeakKey,
    #[error("payload larger than {MAX_PAYLOAD} bytes")]
    TooLarge,
}

// ------------------------------------------------------------ advertisement

/// The service data in a Beam advertisement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BleAdvert {
    pub version: u8,
    pub device_type: DeviceType,
    /// First 6 bytes of the device id, or of the rotating id when
    /// [`caps::ROTATING_ID`] is set.
    pub id: [u8; 6],
    /// [`caps`] bits.
    pub flags: u8,
}

impl BleAdvert {
    pub fn encode(&self) -> [u8; ADVERT_LEN] {
        let mut out = [0u8; ADVERT_LEN];
        out[0] = self.version;
        out[1] = match self.device_type {
            DeviceType::Phone => 0,
            DeviceType::Laptop => 1,
        };
        out[2..8].copy_from_slice(&self.id);
        out[8] = self.flags;
        out
    }

    /// `None` for anything that isn't a Beam advertisement we understand.
    pub fn decode(data: &[u8]) -> Option<BleAdvert> {
        if data.len() < ADVERT_LEN || data[0] != BLE_VERSION {
            return None;
        }
        Some(BleAdvert {
            version: data[0],
            device_type: match data[1] {
                0 => DeviceType::Phone,
                1 => DeviceType::Laptop,
                _ => return None,
            },
            id: data[2..8].try_into().expect("6 bytes"),
            flags: data[8],
        })
    }
}

/// What to advertise: the service data, and the scan-response name (only
/// in "everyone" mode). `None` when off.
pub fn advertisement(
    me: &LocalDevice,
    mode: VisibilityMode,
    capabilities: u8,
) -> Option<(BleAdvert, Option<String>)> {
    let flags = (capabilities & !caps::ROTATING_ID) | caps::ACCEPTING;
    let (id, flags, name) = match mode {
        VisibilityMode::Off => return None,
        VisibilityMode::Everyone => (
            prefix(&me.device_id().0),
            flags,
            Some(truncate(&me.name, MAX_SCAN_NAME_BYTES).to_string()),
        ),
        VisibilityMode::TrustedOnly => (
            prefix(&rotating_id(me.identity.public_key(), current_window())),
            flags | caps::ROTATING_ID,
            None,
        ),
    };
    let advert = BleAdvert {
        version: BLE_VERSION,
        device_type: me.device_type,
        id,
        flags,
    };
    Some((advert, name))
}

fn prefix(bytes: &[u8; 8]) -> [u8; 6] {
    bytes[..6].try_into().expect("6 bytes")
}

/// A device seen over Bluetooth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleDevice {
    /// The advertised id; what a scanner keys the device by while it's in
    /// range. Changes every 15 minutes for trusted-only devices.
    pub id: [u8; 6],
    /// Known when it's a trusted device; strangers only show the 6-byte
    /// prefix until `INFO` is read.
    pub device_id: Option<DeviceId>,
    pub name: String,
    pub device_type: DeviceType,
    pub trusted: Option<TrustedDevice>,
    pub flags: u8,
}

/// Turns an advertisement into a device to show, or `None` for our own, or
/// for a trusted-only device we don't trust.
pub fn interpret_advert(
    advert: &BleAdvert,
    scan_name: Option<&str>,
    me: &LocalDevice,
    trusted: &[TrustedDevice],
) -> Option<BleDevice> {
    if advert.flags & caps::ROTATING_ID != 0 {
        let window = current_window();
        let windows = [window.saturating_sub(1), window, window + 1];
        let matches = |key: &[u8; 32]| {
            windows
                .iter()
                .any(|&w| prefix(&rotating_id(key, w)) == advert.id)
        };
        if matches(me.identity.public_key()) {
            return None;
        }
        let device = trusted.iter().find(|t| matches(&t.public_key))?;
        return Some(BleDevice {
            id: advert.id,
            device_id: Some(device.device_id),
            name: device.name.clone(),
            device_type: advert.device_type,
            trusted: Some(device.clone()),
            flags: advert.flags,
        });
    }
    if prefix(&me.device_id().0) == advert.id {
        return None;
    }
    let known = trusted.iter().find(|t| prefix(&t.device_id.0) == advert.id);
    let name = scan_name
        .map(|n| truncate(n, MAX_SCAN_NAME_BYTES).to_string())
        .or_else(|| known.map(|t| t.name.clone()))
        .unwrap_or_else(|| match advert.device_type {
            DeviceType::Phone => "Phone".to_string(),
            DeviceType::Laptop => "Laptop".to_string(),
        });
    Some(BleDevice {
        id: advert.id,
        device_id: known.map(|t| t.device_id),
        name,
        device_type: advert.device_type,
        trusted: known.cloned(),
        flags: advert.flags,
    })
}

// ------------------------------------------------------------ GATT messages

/// `INFO`, read by the sender. In trusted-only mode the name and
/// fingerprint are left out: the sender already recognized the device from
/// its rotating id, and a stranger learns nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub v: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub device_type: DeviceType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<DeviceId>,
    pub caps: u8,
    /// Fresh for every GATT connection.
    #[serde(with = "serde_bytes")]
    pub eph: [u8; 32],
}

/// A Wi-Fi network one side hosts for the other to join.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotspotInfo {
    pub ssid: String,
    pub passphrase: String,
}

/// How the two devices will connect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// Both are on the same network already.
    Lan,
    /// One joins the other's hotspot.
    Hotspot,
    WifiDirect,
}

/// `CONNECT_REQUEST`, from the sender.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectRequest {
    pub caps: u8,
    /// Links the sender can use, best first.
    pub links: Vec<LinkKind>,
    /// The sender's LAN addresses (`ip`), so the receiver can tell whether
    /// they share a network.
    #[serde(default)]
    pub lan: Vec<String>,
    /// Set when the sender is hosting a hotspot for this transfer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotspot: Option<HotspotInfo>,
}

/// `CONNECT_RESPONSE`, from the receiver: the link it chose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectResponse {
    /// `None` when the receiver declines (`reason` says why).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkKind>,
    /// `ip:port` to connect to over that link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addr: Option<String>,
    /// Set when the receiver hosts a hotspot for the sender to join.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotspot: Option<HotspotInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

fn to_cbor<T: Serialize>(value: &T) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(value, &mut out).expect("CBOR into a Vec can't fail");
    out
}

fn from_cbor<T: for<'de> Deserialize<'de>>(data: &[u8], what: &'static str) -> Result<T, BleError> {
    ciborium::from_reader(data).map_err(|_| BleError::Malformed(what))
}

// ------------------------------------------------------------ GATT crypto

const HKDF_SALT: &[u8] = b"beam 2026-10-29 ble handshake v1";
const AAD_REQUEST: &[u8] = b"beam connect request";
const AAD_RESPONSE: &[u8] = b"beam connect response";

/// Each key seals exactly one message, so a fixed nonce is safe.
const NONCE: [u8; 12] = [0; 12];

fn random_secret() -> StaticSecret {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the OS random number generator failed");
    StaticSecret::from(bytes)
}

/// (sender → receiver key, receiver → sender key).
fn derive_keys(
    secret: &StaticSecret,
    peer: &[u8; 32],
    client: &[u8; 32],
    server: &[u8; 32],
) -> Result<(Key, Key), BleError> {
    let shared = secret.diffie_hellman(&PublicKey::from(*peer));
    if !shared.was_contributory() {
        return Err(BleError::WeakKey);
    }
    let mut info = Vec::with_capacity(64);
    info.extend_from_slice(client);
    info.extend_from_slice(server);
    let mut okm = [0u8; 64];
    Hkdf::<Sha256>::new(Some(HKDF_SALT), shared.as_bytes())
        .expand(&info, &mut okm)
        .expect("64 bytes is a valid HKDF-SHA256 length");
    Ok((*Key::from_slice(&okm[..32]), *Key::from_slice(&okm[32..])))
}

fn seal(key: &Key, aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    ChaCha20Poly1305::new(key)
        .encrypt(
            Nonce::from_slice(&NONCE),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .expect("ChaCha20-Poly1305 sealing can't fail")
}

fn open(key: &Key, aad: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, BleError> {
    ChaCha20Poly1305::new(key)
        .decrypt(
            Nonce::from_slice(&NONCE),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| BleError::Decrypt)
}

/// The receiver's side of one GATT connection: make one per connection,
/// serve [`info`](Self::info), then hand the written request to
/// [`accept_request`](Self::accept_request).
pub struct GattServerHandshake {
    secret: StaticSecret,
    public: [u8; 32],
}

impl Default for GattServerHandshake {
    fn default() -> Self {
        Self::new()
    }
}

impl GattServerHandshake {
    pub fn new() -> GattServerHandshake {
        let secret = random_secret();
        let public = PublicKey::from(&secret).to_bytes();
        GattServerHandshake { secret, public }
    }

    /// The `INFO` value for this connection.
    pub fn info(&self, me: &LocalDevice, mode: VisibilityMode, capabilities: u8) -> Vec<u8> {
        let open = mode == VisibilityMode::Everyone;
        to_cbor(&DeviceInfo {
            v: BLE_VERSION,
            name: open.then(|| truncate(&me.name, 63).to_string()),
            device_type: me.device_type,
            fingerprint: open.then(|| me.device_id()),
            caps: capabilities,
            eph: self.public,
        })
    }

    /// Decrypts a `CONNECT_REQUEST`. The returned sealer encrypts the one
    /// `CONNECT_RESPONSE`.
    pub fn accept_request(self, data: &[u8]) -> Result<(ConnectRequest, ResponseSealer), BleError> {
        if data.len() < 32 {
            return Err(BleError::Malformed("connect request"));
        }
        if data.len() > MAX_PAYLOAD {
            return Err(BleError::TooLarge);
        }
        let client: [u8; 32] = data[..32].try_into().expect("32 bytes");
        let (to_server, to_client) = derive_keys(&self.secret, &client, &client, &self.public)?;
        let plaintext = open(&to_server, AAD_REQUEST, &data[32..])?;
        let request = from_cbor(&plaintext, "connect request")?;
        Ok((request, ResponseSealer { key: to_client }))
    }
}

/// Encrypts the receiver's `CONNECT_RESPONSE`.
pub struct ResponseSealer {
    key: Key,
}

impl ResponseSealer {
    pub fn seal(self, response: &ConnectResponse) -> Vec<u8> {
        seal(&self.key, AAD_RESPONSE, &to_cbor(response))
    }
}

/// The sender's side: read `INFO`, write the request, open the response.
pub struct GattClientHandshake;

impl GattClientHandshake {
    /// Parses `INFO` and seals `request` for that receiver. Write the
    /// returned bytes to `CONNECT_REQUEST`, then open the notified
    /// `CONNECT_RESPONSE` with the returned opener.
    pub fn request(
        info: &[u8],
        request: &ConnectRequest,
    ) -> Result<(DeviceInfo, Vec<u8>, ResponseOpener), BleError> {
        if info.len() > MAX_PAYLOAD {
            return Err(BleError::TooLarge);
        }
        let info: DeviceInfo = from_cbor(info, "device info")?;
        if info.v != BLE_VERSION {
            return Err(BleError::Version(info.v));
        }
        let secret = random_secret();
        let public = PublicKey::from(&secret).to_bytes();
        let (to_server, to_client) = derive_keys(&secret, &info.eph, &public, &info.eph)?;
        let mut data = public.to_vec();
        data.extend(seal(&to_server, AAD_REQUEST, &to_cbor(request)));
        Ok((info, data, ResponseOpener { key: to_client }))
    }
}

/// Decrypts the receiver's `CONNECT_RESPONSE`.
pub struct ResponseOpener {
    key: Key,
}

impl ResponseOpener {
    pub fn open(self, data: &[u8]) -> Result<ConnectResponse, BleError> {
        if data.len() > MAX_PAYLOAD {
            return Err(BleError::TooLarge);
        }
        from_cbor(&open(&self.key, AAD_RESPONSE, data)?, "connect response")
    }
}

// ------------------------------------------------------------ link choice

/// Our address on a network the sender is also on, judged by IPv4 /24 (the
/// usual home and hotspot subnet), so the receiver can answer "use the LAN"
/// instead of setting up a hotspot. `None` when nothing matches.
pub fn shared_lan(theirs: &[IpAddr], ours: &[IpAddr]) -> Option<IpAddr> {
    ours.iter().copied().find(|mine| match mine {
        IpAddr::V4(m) if !m.is_loopback() && !m.is_link_local() => theirs.iter().any(|t| match t {
            IpAddr::V4(t) => t.octets()[..3] == m.octets()[..3] && t != m,
            IpAddr::V6(_) => false,
        }),
        _ => false,
    })
}

// ------------------------------------------------------------ fragmentation

/// Bytes of ATT overhead in a write or notification.
const ATT_OVERHEAD: usize = 3;
/// Each fragment starts with its index and the fragment count.
const FRAGMENT_HEADER: usize = 2;

/// Splits `payload` into fragments that each fit one ATT write or
/// notification at `mtu`: `[index, count] ‖ chunk`.
pub fn fragment(payload: &[u8], mtu: usize) -> Result<Vec<Vec<u8>>, BleError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(BleError::TooLarge);
    }
    let room = mtu.saturating_sub(ATT_OVERHEAD + FRAGMENT_HEADER).max(1);
    let chunks: Vec<&[u8]> = if payload.is_empty() {
        vec![&[]]
    } else {
        payload.chunks(room).collect()
    };
    let count = u8::try_from(chunks.len()).map_err(|_| BleError::TooLarge)?;
    Ok(chunks
        .into_iter()
        .enumerate()
        .map(|(i, chunk)| {
            let mut out = Vec::with_capacity(chunk.len() + FRAGMENT_HEADER);
            out.push(i as u8);
            out.push(count);
            out.extend_from_slice(chunk);
            out
        })
        .collect())
}

/// Puts fragments back together, in any order; a repeated fragment
/// replaces the earlier copy.
#[derive(Debug, Default)]
pub struct Reassembler {
    count: Option<u8>,
    parts: HashMap<u8, Vec<u8>>,
    size: usize,
}

impl Reassembler {
    /// Adds a fragment; returns the whole payload once every fragment is in.
    pub fn push(&mut self, fragment: &[u8]) -> Result<Option<Vec<u8>>, BleError> {
        let [index, count, chunk @ ..] = fragment else {
            return Err(BleError::Malformed("fragment"));
        };
        if *count == 0 || index >= count || self.count.is_some_and(|c| c != *count) {
            return Err(BleError::Malformed("fragment"));
        }
        self.count = Some(*count);
        if let Some(old) = self.parts.insert(*index, chunk.to_vec()) {
            self.size -= old.len();
        }
        self.size += chunk.len();
        if self.size > MAX_PAYLOAD {
            return Err(BleError::TooLarge);
        }
        if self.parts.len() < *count as usize {
            return Ok(None);
        }
        let payload = (0..*count)
            .flat_map(|i| self.parts.remove(&i).expect("all parts present"))
            .collect();
        *self = Reassembler::default();
        Ok(Some(payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    fn device(name: &str, device_type: DeviceType) -> LocalDevice {
        LocalDevice {
            identity: Identity::generate(),
            name: name.into(),
            device_type,
        }
    }

    fn trusted(device: &LocalDevice) -> TrustedDevice {
        TrustedDevice::new(device.name.clone(), *device.identity.public_key(), false)
    }

    #[test]
    fn advertisement_round_trips_in_ten_bytes() {
        let laptop = device("ThinkPad", DeviceType::Laptop);
        let (advert, name) = advertisement(&laptop, VisibilityMode::Everyone, caps::LAN).unwrap();
        let bytes = advert.encode();
        assert_eq!(bytes.len(), ADVERT_LEN);
        assert_eq!(bytes[0], BLE_VERSION);
        assert_eq!(bytes[1], 1);
        assert_eq!(&bytes[2..8], &laptop.device_id().0[..6]);
        assert_eq!(bytes[8], caps::LAN | caps::ACCEPTING);
        assert_eq!(BleAdvert::decode(&bytes), Some(advert));
        assert_eq!(name.as_deref(), Some("ThinkPad"));
    }

    #[test]
    fn foreign_or_future_adverts_are_ignored() {
        assert_eq!(BleAdvert::decode(&[1, 0, 0]), None);
        assert_eq!(BleAdvert::decode(&[2, 0, 1, 2, 3, 4, 5, 6, 0, 0]), None);
        assert_eq!(BleAdvert::decode(&[1, 7, 1, 2, 3, 4, 5, 6, 0, 0]), None);
    }

    #[test]
    fn everyone_shows_the_name_and_hides_ourselves() {
        let phone = device("Pixel", DeviceType::Phone);
        let laptop = device("ThinkPad", DeviceType::Laptop);
        let (advert, name) = advertisement(&laptop, VisibilityMode::Everyone, 0).unwrap();

        let seen = interpret_advert(&advert, name.as_deref(), &phone, &[]).unwrap();
        assert_eq!(seen.name, "ThinkPad");
        assert_eq!(seen.device_type, DeviceType::Laptop);
        assert_eq!(seen.device_id, None);
        assert!(seen.trusted.is_none());

        let known = interpret_advert(&advert, None, &phone, &[trusted(&laptop)]).unwrap();
        assert_eq!(known.device_id, Some(laptop.device_id()));
        assert_eq!(known.name, "ThinkPad");

        assert!(interpret_advert(&advert, name.as_deref(), &laptop, &[]).is_none());
    }

    #[test]
    fn trusted_only_is_anonymous_to_strangers() {
        let phone = device("Pixel", DeviceType::Phone);
        let laptop = device("ThinkPad", DeviceType::Laptop);
        let stranger = device("Someone", DeviceType::Phone);
        let (advert, name) = advertisement(&laptop, VisibilityMode::TrustedOnly, 0).unwrap();
        assert!(name.is_none(), "no name in the scan response");
        assert_ne!(advert.id, prefix(&laptop.device_id().0));
        assert!(advert.flags & caps::ROTATING_ID != 0);

        assert!(interpret_advert(&advert, None, &stranger, &[]).is_none());
        let seen = interpret_advert(&advert, None, &phone, &[trusted(&laptop)]).unwrap();
        assert_eq!(seen.device_id, Some(laptop.device_id()));
        assert!(interpret_advert(&advert, None, &laptop, &[]).is_none());
    }

    #[test]
    fn off_advertises_nothing() {
        let laptop = device("ThinkPad", DeviceType::Laptop);
        assert!(advertisement(&laptop, VisibilityMode::Off, caps::LAN).is_none());
    }

    #[test]
    fn long_names_fit_the_scan_response() {
        let laptop = device(&"é".repeat(40), DeviceType::Laptop);
        let (_, name) = advertisement(&laptop, VisibilityMode::Everyone, 0).unwrap();
        let name = name.unwrap();
        assert!(name.len() <= MAX_SCAN_NAME_BYTES);
        assert!(name.chars().all(|c| c == 'é'));
    }

    fn hotspot_request() -> ConnectRequest {
        ConnectRequest {
            caps: caps::HOTSPOT,
            links: vec![LinkKind::Lan, LinkKind::Hotspot],
            lan: vec![],
            hotspot: Some(HotspotInfo {
                ssid: "DIRECT-beam-7f3a".into(),
                passphrase: "correct horse battery".into(),
            }),
        }
    }

    #[test]
    fn handshake_carries_the_request_and_response_privately() {
        let laptop = device("ThinkPad", DeviceType::Laptop);
        let server = GattServerHandshake::new();
        let info = server.info(&laptop, VisibilityMode::Everyone, caps::LAN);

        let (seen, request_bytes, opener) =
            GattClientHandshake::request(&info, &hotspot_request()).unwrap();
        assert_eq!(seen.name.as_deref(), Some("ThinkPad"));
        assert_eq!(seen.fingerprint, Some(laptop.device_id()));
        // The Wi-Fi password never crosses the air in the clear.
        let needle = b"correct horse battery";
        assert!(!request_bytes.windows(needle.len()).any(|w| w == needle));

        let (request, sealer) = server.accept_request(&request_bytes).unwrap();
        assert_eq!(request, hotspot_request());

        let response = ConnectResponse {
            link: Some(LinkKind::Hotspot),
            addr: Some("192.168.49.1:40000".into()),
            hotspot: None,
            reason: None,
        };
        assert_eq!(opener.open(&sealer.seal(&response)).unwrap(), response);
    }

    #[test]
    fn trusted_only_info_reveals_no_identity() {
        let laptop = device("ThinkPad", DeviceType::Laptop);
        let info = GattServerHandshake::new().info(&laptop, VisibilityMode::TrustedOnly, 0);
        let (seen, _, _) = GattClientHandshake::request(&info, &hotspot_request()).unwrap();
        assert_eq!(seen.name, None);
        assert_eq!(seen.fingerprint, None);
        let needle = laptop.name.as_bytes();
        assert!(!info.windows(needle.len()).any(|w| w == needle));
    }

    #[test]
    fn tampering_and_wrong_keys_are_caught() {
        let laptop = device("ThinkPad", DeviceType::Laptop);
        let server = GattServerHandshake::new();
        let info = server.info(&laptop, VisibilityMode::Everyone, 0);
        let (_, mut request_bytes, _) =
            GattClientHandshake::request(&info, &hotspot_request()).unwrap();
        let last = request_bytes.len() - 1;
        request_bytes[last] ^= 1;
        assert_eq!(
            server.accept_request(&request_bytes).err(),
            Some(BleError::Decrypt)
        );

        // A request sealed for another connection's key.
        let other = GattServerHandshake::new().info(&laptop, VisibilityMode::Everyone, 0);
        let (_, for_other, _) = GattClientHandshake::request(&other, &hotspot_request()).unwrap();
        let server = GattServerHandshake::new();
        assert_eq!(
            server.accept_request(&for_other).err(),
            Some(BleError::Decrypt)
        );
    }

    #[test]
    fn a_low_order_key_is_refused() {
        let server = GattServerHandshake::new();
        let mut data = vec![0u8; 32];
        data.extend([0u8; 40]);
        assert_eq!(server.accept_request(&data).err(), Some(BleError::WeakKey));
    }

    #[test]
    fn info_from_a_newer_version_is_refused() {
        let laptop = device("ThinkPad", DeviceType::Laptop);
        let server = GattServerHandshake::new();
        let mut info: DeviceInfo =
            from_cbor(&server.info(&laptop, VisibilityMode::Everyone, 0), "info").unwrap();
        info.v = 2;
        let result = GattClientHandshake::request(&to_cbor(&info), &hotspot_request());
        assert_eq!(result.err(), Some(BleError::Version(2)));
    }

    #[test]
    fn a_shared_lan_is_found_by_subnet() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let ours = [ip("127.0.0.1"), ip("10.0.0.5"), ip("192.168.1.20")];
        assert_eq!(
            shared_lan(&[ip("192.168.1.31")], &ours),
            Some(ip("192.168.1.20"))
        );
        assert_eq!(shared_lan(&[ip("192.168.2.31")], &ours), None);
        assert_eq!(
            shared_lan(&[ip("127.0.0.9")], &ours),
            None,
            "loopback never counts"
        );
        assert_eq!(shared_lan(&[], &ours), None);
    }

    #[test]
    fn fragments_reassemble_in_any_order() {
        let payload: Vec<u8> = (0..700u32).map(|i| i as u8).collect();
        for mtu in [23, 185, PREFERRED_MTU] {
            let mut parts = fragment(&payload, mtu).unwrap();
            assert!(parts.iter().all(|p| p.len() <= mtu - ATT_OVERHEAD));
            parts.reverse();
            let duplicate = parts[0].clone();
            parts.insert(1, duplicate);
            let mut reassembler = Reassembler::default();
            let mut whole = None;
            for part in &parts {
                if let Some(done) = reassembler.push(part).unwrap() {
                    whole = Some(done);
                }
            }
            assert_eq!(whole.as_deref(), Some(&payload[..]), "mtu {mtu}");
        }
    }

    #[test]
    fn bad_fragments_are_errors() {
        let mut r = Reassembler::default();
        assert!(r.push(&[0]).is_err());
        assert!(r.push(&[2, 2, 0]).is_err(), "index past count");
        assert!(r.push(&[0, 0]).is_err(), "zero count");
        assert!(r.push(&[0, 3, 1]).unwrap().is_none());
        assert!(r.push(&[1, 4, 1]).is_err(), "count changed");
        assert_eq!(
            fragment(&vec![0; MAX_PAYLOAD + 1], 517),
            Err(BleError::TooLarge)
        );
        assert_eq!(fragment(&[], 23).unwrap(), vec![vec![0, 1]]);
    }

    #[test]
    fn oversized_reassembly_is_refused() {
        let mut r = Reassembler::default();
        let big = vec![0u8; 250];
        let mut result = Ok(None);
        for i in 0..20u8 {
            let mut part = vec![i, 20];
            part.extend(&big);
            result = r.push(&part);
            if result.is_err() {
                break;
            }
        }
        assert_eq!(result, Err(BleError::TooLarge));
    }
}
