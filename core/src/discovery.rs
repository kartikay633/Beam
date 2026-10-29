//! Finding devices on the same network with mDNS / DNS-SD (SPEC.md 4.1).
//!
//! A receiver announces `_beam._tcp.local.` according to its visibility:
//!
//! - **everyone**: TXT `v`, `id` (device id, 16 hex digits), `name`, `type`
//!   and `vis=everyone`.
//! - **trusted only**: TXT `v`, `vis=trusted` and `rid`, a *rotating
//!   identifier* derived from the device's public key and the current
//!   15-minute window. No name, no id. A device that trusts it knows its
//!   public key, so it can compute the same `rid` and recognize it; anyone
//!   else sees an anonymous entry that changes every 15 minutes.
//! - **off**: nothing.
//!
//! Instance and host names are built from the id (or the `rid`), so the
//! computer's own host name is never broadcast.
//!
//! [`Advertiser`] and [`Browser`] do mDNS with the `mdns-sd` crate. A
//! platform with its own mDNS stack (Android's `NsdManager`) instead asks
//! [`announcement`] what to register, re-registers at
//! [`next_announcement_change`], and hands resolved services to
//! [`interpret_service`], so the TXT format and the rotating identifier stay
//! in one place.
//!
//! mDNS is unauthenticated: anyone can announce any name. A device found
//! here is only a *hint*; the sender then checks, through the Noise handshake,
//! that it reached the device id it was looking for.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::sync::mpsc;

use crate::messages::{DeviceId, DeviceType, PROTOCOL_VERSION};
use crate::transfer::LocalDevice;
use crate::trust::{TrustStore, TrustedDevice};
use crate::visibility::{Visibility, VisibilityMode};

pub const SERVICE_TYPE: &str = "_beam._tcp.local.";
/// How often the trusted-only identifier changes.
pub const ROTATION: Duration = Duration::from_secs(15 * 60);
const ROTATING_ID_CONTEXT: &str = "beam 2026-10-27 rotating id v1";
/// TXT values are limited to 255 bytes; names are cut well before that.
const MAX_NAME_BYTES: usize = 63;

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("mDNS failed: {0}")]
    Mdns(String),
    #[error("no device called {0:?} was found on the local network")]
    NotFound(String),
    #[error("{query:?} matches several devices: {}; use the device id instead", .matches.join(", "))]
    Ambiguous { query: String, matches: Vec<String> },
    #[error("found {0}, but none of its addresses accepted a connection")]
    Unreachable(String),
}

impl From<mdns_sd::Error> for DiscoveryError {
    fn from(e: mdns_sd::Error) -> Self {
        DiscoveryError::Mdns(e.to_string())
    }
}

/// A device seen on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearbyDevice {
    pub device_id: DeviceId,
    pub name: String,
    /// Not announced in trusted-only mode.
    pub device_type: Option<DeviceType>,
    pub addresses: Vec<SocketAddr>,
    /// Our trust-store entry for it, if any.
    pub trusted: Option<TrustedDevice>,
    pub visibility: VisibilityMode,
}

/// The 8-byte identifier a device announces in trusted-only mode during
/// rotation window `window` (Unix time / 15 minutes).
pub fn rotating_id(public_key: &[u8; 32], window: u64) -> [u8; 8] {
    let key = blake3::derive_key(ROTATING_ID_CONTEXT, public_key);
    let hash = blake3::keyed_hash(&key, &window.to_le_bytes());
    hash.as_bytes()[..8].try_into().expect("8 bytes")
}

/// The current rotation window.
pub fn current_window() -> u64 {
    crate::receiver::now_unix() / ROTATION.as_secs()
}

/// What a device announces, as decoded from its TXT record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Announcement {
    Everyone {
        id: DeviceId,
        name: String,
        device_type: DeviceType,
    },
    TrustedOnly {
        rid: [u8; 8],
    },
}

impl Announcement {
    /// The announcement for `me` in `mode`, or `None` when off.
    pub(crate) fn of(me: &LocalDevice, mode: VisibilityMode, window: u64) -> Option<Announcement> {
        match mode {
            VisibilityMode::Off => None,
            VisibilityMode::TrustedOnly => Some(Announcement::TrustedOnly {
                rid: rotating_id(me.identity.public_key(), window),
            }),
            VisibilityMode::Everyone => Some(Announcement::Everyone {
                id: me.device_id(),
                name: truncate(&me.name, MAX_NAME_BYTES).to_string(),
                device_type: me.device_type,
            }),
        }
    }

    /// Instance label (also used as the host label), never the real host name.
    pub(crate) fn instance_name(&self) -> String {
        match self {
            Announcement::Everyone { id, .. } => format!("beam-{}", hex::encode(id.0)),
            Announcement::TrustedOnly { rid } => format!("beam-{}", hex::encode(rid)),
        }
    }

    pub(crate) fn txt(&self) -> Vec<(String, String)> {
        let mut txt = vec![("v".to_string(), PROTOCOL_VERSION.major.to_string())];
        match self {
            Announcement::Everyone {
                id,
                name,
                device_type,
            } => {
                txt.push(("vis".into(), VisibilityMode::Everyone.as_str().into()));
                txt.push(("id".into(), hex::encode(id.0)));
                txt.push(("name".into(), name.clone()));
                txt.push((
                    "type".into(),
                    match device_type {
                        DeviceType::Phone => "phone".into(),
                        DeviceType::Laptop => "laptop".into(),
                    },
                ));
            }
            Announcement::TrustedOnly { rid } => {
                txt.push(("vis".into(), VisibilityMode::TrustedOnly.as_str().into()));
                txt.push(("rid".into(), hex::encode(rid)));
            }
        }
        txt
    }

    /// Decodes a TXT record. Anything malformed, or from another major
    /// protocol version, is ignored.
    pub(crate) fn parse(txt: &HashMap<String, String>) -> Option<Announcement> {
        if txt.get("v")? != &PROTOCOL_VERSION.major.to_string() {
            return None;
        }
        match txt.get("vis")?.as_str() {
            "everyone" => Some(Announcement::Everyone {
                id: txt.get("id")?.parse().ok()?,
                name: truncate(txt.get("name")?, MAX_NAME_BYTES).to_string(),
                device_type: match txt.get("type")?.as_str() {
                    "phone" => DeviceType::Phone,
                    "laptop" => DeviceType::Laptop,
                    _ => return None,
                },
            }),
            "trusted" => Some(Announcement::TrustedOnly {
                rid: hex::decode(txt.get("rid")?).ok()?.try_into().ok()?,
            }),
            _ => None,
        }
    }

    /// Turns an announcement into a device we can show, or `None` if it's
    /// our own, or a trusted-only device we don't trust (and so can't
    /// recognize).
    pub(crate) fn interpret(
        &self,
        addresses: Vec<SocketAddr>,
        my_key: &[u8; 32],
        trusted: &[TrustedDevice],
        window: u64,
    ) -> Option<NearbyDevice> {
        match self {
            Announcement::Everyone {
                id,
                name,
                device_type,
            } => {
                if *id == DeviceId::from_public_key(my_key) {
                    return None;
                }
                Some(NearbyDevice {
                    device_id: *id,
                    name: name.clone(),
                    device_type: Some(*device_type),
                    addresses,
                    trusted: trusted.iter().find(|t| t.device_id == *id).cloned(),
                    visibility: VisibilityMode::Everyone,
                })
            }
            Announcement::TrustedOnly { rid } => {
                // Allow one window of clock difference either way.
                let windows = [window.saturating_sub(1), window, window + 1];
                if windows.iter().any(|&w| rotating_id(my_key, w) == *rid) {
                    return None;
                }
                let device = trusted.iter().find(|t| {
                    windows
                        .iter()
                        .any(|&w| rotating_id(&t.public_key, w) == *rid)
                })?;
                Some(NearbyDevice {
                    device_id: device.device_id,
                    name: device.name.clone(),
                    device_type: None,
                    addresses,
                    trusted: Some(device.clone()),
                    visibility: VisibilityMode::TrustedOnly,
                })
            }
        }
    }
}

pub(crate) fn truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Announces this device on the local network while it exists, following
/// visibility changes, the "everyone" expiry and the 15-minute rotation.
/// A service to register with the platform's mDNS stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceAnnouncement {
    /// Instance name, e.g. `beam-1a2b3c4d5e6f7a8b`.
    pub instance_name: String,
    pub txt: Vec<(String, String)>,
}

/// What `me` should announce in `mode` right now, or `None` when off.
pub fn announcement(me: &LocalDevice, mode: VisibilityMode) -> Option<ServiceAnnouncement> {
    Announcement::of(me, mode, current_window()).map(|a| ServiceAnnouncement {
        instance_name: a.instance_name(),
        txt: a.txt(),
    })
}

/// When the announcement may next change without anyone touching the
/// visibility: the end of "everyone", or the next rotation window.
pub fn next_announcement_change(visibility: &Visibility) -> tokio::time::Instant {
    let now = crate::receiver::now_unix();
    let next = (now / ROTATION.as_secs() + 1) * ROTATION.as_secs();
    let next_window = tokio::time::Instant::now() + Duration::from_secs(next - now);
    match (visibility.mode(), visibility.everyone_until()) {
        (VisibilityMode::Everyone, Some(until)) => until.min(next_window),
        _ => next_window,
    }
}

/// Turns a service the platform resolved (its TXT record and addresses)
/// into a device to show. `None` for our own announcement, anything
/// malformed, and trusted-only devices we don't trust.
pub fn interpret_service(
    txt: &HashMap<String, String>,
    ips: impl IntoIterator<Item = IpAddr>,
    port: u16,
    me: &LocalDevice,
    trusted: &[TrustedDevice],
) -> Option<NearbyDevice> {
    let addresses = socket_addrs(ips.into_iter(), port);
    Announcement::parse(txt)?.interpret(
        addresses,
        me.identity.public_key(),
        trusted,
        current_window(),
    )
}

/// How often to look for a new network (another Wi-Fi, a hotspot).
const NETWORK_CHECK: Duration = Duration::from_secs(3);

/// This machine's IPv4 addresses, sorted, to notice when the network
/// changes. `mdns-sd` follows address changes itself, but missed a laptop
/// moving from one Wi-Fi network to another and kept announcing the old
/// addresses only, so the daemon is replaced when these change.
fn network_ipv4s() -> Vec<IpAddr> {
    let mut ips: Vec<IpAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .map(|i| i.ip())
        .filter(IpAddr::is_ipv4)
        .collect();
    ips.sort();
    ips.dedup();
    ips
}

/// The current daemon, and whether its owner has shut it down for good.
type SharedDaemon = std::sync::Arc<std::sync::Mutex<(ServiceDaemon, bool)>>;

fn shared(daemon: ServiceDaemon) -> SharedDaemon {
    std::sync::Arc::new(std::sync::Mutex::new((daemon, false)))
}

fn shut_down(daemon: &SharedDaemon) {
    let mut guard = daemon.lock().unwrap_or_else(|e| e.into_inner());
    guard.1 = true;
    let _ = guard.0.shutdown();
}

/// Swaps in `fresh` unless the owner has shut down, and shuts down the old one.
fn replace(daemon: &SharedDaemon, fresh: ServiceDaemon) -> bool {
    let mut guard = daemon.lock().unwrap_or_else(|e| e.into_inner());
    if guard.1 {
        let _ = fresh.shutdown();
        return false;
    }
    let old = std::mem::replace(&mut guard.0, fresh);
    let _ = old.shutdown();
    true
}

fn current_daemon(daemon: &SharedDaemon) -> ServiceDaemon {
    daemon.lock().unwrap_or_else(|e| e.into_inner()).0.clone()
}

pub struct Advertiser {
    task: tokio::task::JoinHandle<()>,
    daemon: SharedDaemon,
}

impl Advertiser {
    pub fn start(
        me: LocalDevice,
        port: u16,
        visibility: Visibility,
    ) -> Result<Advertiser, DiscoveryError> {
        let daemon = shared(ServiceDaemon::new()?);
        let task = tokio::spawn(advertise(daemon.clone(), me, port, visibility));
        Ok(Advertiser { task, daemon })
    }
}

impl Drop for Advertiser {
    fn drop(&mut self) {
        self.task.abort();
        shut_down(&self.daemon);
    }
}

async fn advertise(shared: SharedDaemon, me: LocalDevice, port: u16, visibility: Visibility) {
    let mut changes = visibility.subscribe();
    let mut current: Option<(Announcement, String)> = None;
    let mut network = network_ipv4s();
    loop {
        let daemon = current_daemon(&shared);
        let mode = visibility.mode();
        let wanted = Announcement::of(&me, mode, current_window());
        if wanted.as_ref() != current.as_ref().map(|(a, _)| a) {
            if let Some((_, fullname)) = current.take() {
                let _ = daemon.unregister(&fullname);
            }
            if let Some(announcement) = wanted {
                let instance = announcement.instance_name();
                let info = ServiceInfo::new(
                    SERVICE_TYPE,
                    &instance,
                    &format!("{instance}.local."),
                    "",
                    port,
                    &announcement.txt()[..],
                )
                .map(ServiceInfo::enable_addr_auto);
                if let Ok(info) = info {
                    let fullname = info.get_fullname().to_string();
                    if daemon.register(info).is_ok() {
                        current = Some((announcement, fullname));
                    }
                }
            }
        }

        // Wake for an explicit change, the end of "everyone", the next
        // rotation window, or a new network, whichever comes first.
        let deadline = next_announcement_change(&visibility);
        loop {
            tokio::select! {
                _ = changes.changed() => break,
                _ = tokio::time::sleep_until(deadline) => break,
                _ = tokio::time::sleep(NETWORK_CHECK) => {}
            }
            let now = network_ipv4s();
            if now != network {
                network = now;
                if let Some((_, fullname)) = current.take() {
                    let _ = daemon.unregister(&fullname);
                }
                if let Ok(fresh) = ServiceDaemon::new() {
                    if !replace(&shared, fresh) {
                        return;
                    }
                }
                break;
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    Found(NearbyDevice),
    Lost(DeviceId),
}

/// Watches the local network for Beam devices.
pub struct Browser {
    events: mpsc::UnboundedReceiver<DiscoveryEvent>,
    daemon: SharedDaemon,
}

impl Browser {
    /// Starts browsing. `me` is used to hide our own announcement; `trust`
    /// to recognize trusted devices, including trusted-only ones.
    pub fn start(me: &LocalDevice, trust: TrustStore) -> Result<Browser, DiscoveryError> {
        let daemon = ServiceDaemon::new()?;
        let mut receiver = daemon.browse(SERVICE_TYPE)?;
        let daemon = shared(daemon);
        let (tx, events) = mpsc::unbounded_channel();
        let my_key = *me.identity.public_key();
        let owner = daemon.clone();
        // The mdns-sd receiver is blocking; pump it from a thread.
        std::thread::spawn(move || {
            let mut by_fullname: HashMap<String, DeviceId> = HashMap::new();
            let mut network = network_ipv4s();
            let mut checked = std::time::Instant::now();
            loop {
                if checked.elapsed() >= NETWORK_CHECK {
                    checked = std::time::Instant::now();
                    let now = network_ipv4s();
                    if now != network {
                        network = now;
                        // Browse afresh on the new network. What we saw on
                        // the old one is gone; it'll be found again if not.
                        let Ok(fresh) = ServiceDaemon::new() else {
                            continue;
                        };
                        let Ok(browsing) = fresh.browse(SERVICE_TYPE) else {
                            let _ = fresh.shutdown();
                            continue;
                        };
                        if !replace(&owner, fresh) {
                            break;
                        }
                        receiver = browsing;
                        for (_, id) in by_fullname.drain() {
                            if tx.send(DiscoveryEvent::Lost(id)).is_err() {
                                return;
                            }
                        }
                    }
                }
                let event = match receiver.recv_timeout(Duration::from_millis(500)) {
                    Ok(event) => event,
                    Err(_) if receiver.is_disconnected() => break,
                    Err(_) => continue,
                };
                let out = match event {
                    ServiceEvent::ServiceResolved(service) => {
                        let txt: HashMap<String, String> = service
                            .txt_properties
                            .iter()
                            .map(|p| (p.key().to_string(), p.val_str().to_string()))
                            .collect();
                        let addresses = socket_addrs(
                            service.addresses.iter().map(|a| a.to_ip_addr()),
                            service.port,
                        );
                        let trusted = trust.list().unwrap_or_default();
                        Announcement::parse(&txt)
                            .and_then(|a| {
                                a.interpret(addresses, &my_key, &trusted, current_window())
                            })
                            .map(|device| {
                                by_fullname.insert(service.fullname.clone(), device.device_id);
                                DiscoveryEvent::Found(device)
                            })
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        by_fullname.remove(&fullname).map(DiscoveryEvent::Lost)
                    }
                    _ => None,
                };
                if let Some(event) = out {
                    if tx.send(event).is_err() {
                        break;
                    }
                }
            }
        });
        Ok(Browser { events, daemon })
    }

    /// The next device found or lost.
    pub async fn next(&mut self) -> Option<DiscoveryEvent> {
        self.events.recv().await
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        shut_down(&self.daemon);
    }
}

/// Addresses to try, best first: IPv4 before IPv6, then link-local, and
/// loopback last. (A device announces its loopback address too; from another
/// machine, 127.0.0.1 would reach *this* machine instead.)
fn socket_addrs(ips: impl Iterator<Item = IpAddr>, port: u16) -> Vec<SocketAddr> {
    let mut ips: Vec<IpAddr> = ips.collect::<HashSet<_>>().into_iter().collect();
    ips.sort_by_key(|ip| match ip {
        _ if ip.is_loopback() => (4, *ip),
        IpAddr::V4(v4) if v4.is_link_local() => (2, *ip),
        IpAddr::V4(_) => (0, *ip),
        IpAddr::V6(v6) if v6.segments()[0] & 0xffc0 == 0xfe80 => (3, *ip),
        IpAddr::V6(_) => (1, *ip),
    });
    ips.into_iter()
        .map(|ip| SocketAddr::new(ip, port))
        .collect()
}

/// Finds the device called `query` (its name, ignoring case, or its device
/// id, with or without dashes) among `devices`.
pub fn match_query<'a>(
    devices: &'a [NearbyDevice],
    query: &str,
) -> Result<&'a NearbyDevice, DiscoveryError> {
    if let Ok(id) = query.parse::<DeviceId>() {
        if let Some(device) = devices.iter().find(|d| d.device_id == id) {
            return Ok(device);
        }
    }
    let by_name: Vec<&NearbyDevice> = devices
        .iter()
        .filter(|d| d.name.eq_ignore_ascii_case(query.trim()))
        .collect();
    match by_name.as_slice() {
        [one] => Ok(one),
        [] => Err(DiscoveryError::NotFound(query.to_string())),
        many => Err(DiscoveryError::Ambiguous {
            query: query.to_string(),
            matches: many
                .iter()
                .map(|d| format!("{} ({})", d.name, d.device_id))
                .collect(),
        }),
    }
}

/// Looks for `query` on the network for up to `timeout`. Once a match shows
/// up, waits a moment more so a second device with the same name is noticed.
pub async fn resolve(
    query: &str,
    me: &LocalDevice,
    trust: TrustStore,
    timeout: Duration,
) -> Result<NearbyDevice, DiscoveryError> {
    const SETTLE: Duration = Duration::from_millis(500);
    let mut browser = Browser::start(me, trust)?;
    let mut devices: Vec<NearbyDevice> = Vec::new();
    let mut deadline = tokio::time::Instant::now() + timeout;
    let mut matched = false;
    loop {
        match tokio::time::timeout_at(deadline, browser.next()).await {
            Ok(Some(DiscoveryEvent::Found(device))) => {
                devices.retain(|d| d.device_id != device.device_id);
                devices.push(device);
                if !matched && match_query(&devices, query).is_ok() {
                    matched = true;
                    deadline = deadline.min(tokio::time::Instant::now() + SETTLE);
                }
            }
            Ok(Some(DiscoveryEvent::Lost(id))) => devices.retain(|d| d.device_id != id),
            Ok(None) | Err(_) => break,
        }
    }
    match_query(&devices, query).cloned()
}

/// The first of `addresses` that accepts a TCP connection within 1.5 s
/// (SPEC.md section 5), tried in order.
pub async fn first_reachable(addresses: &[SocketAddr]) -> Option<SocketAddr> {
    for &addr in addresses {
        let attempt = tokio::time::timeout(
            Duration::from_millis(1500),
            tokio::net::TcpStream::connect(addr),
        )
        .await;
        if let Ok(Ok(_)) = attempt {
            return Some(addr);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    fn device(name: &str) -> LocalDevice {
        LocalDevice {
            identity: Identity::generate(),
            name: name.into(),
            device_type: DeviceType::Laptop,
        }
    }

    fn txt_map(announcement: &Announcement) -> HashMap<String, String> {
        announcement.txt().into_iter().collect()
    }

    fn addr() -> Vec<SocketAddr> {
        vec!["192.168.1.20:9000".parse().unwrap()]
    }

    fn trusted(device: &LocalDevice) -> TrustedDevice {
        TrustedDevice::new(device.name.clone(), *device.identity.public_key(), false)
    }

    #[test]
    fn platform_services_use_the_same_announcements() {
        let phone = device("Pixel");
        let laptop = device("ThinkPad");

        let open = announcement(&laptop, VisibilityMode::Everyone).unwrap();
        assert_eq!(
            open.instance_name,
            format!("beam-{}", hex::encode(laptop.device_id().0))
        );
        let txt: HashMap<String, String> = open.txt.into_iter().collect();
        let ip: IpAddr = "192.168.1.20".parse().unwrap();
        let found = interpret_service(&txt, [ip], 9000, &phone, &[]).unwrap();
        assert_eq!(found.device_id, laptop.device_id());
        assert_eq!(found.addresses, vec![SocketAddr::new(ip, 9000)]);
        // Our own announcement, seen by the platform browser, is hidden.
        assert!(interpret_service(&txt, [ip], 9000, &laptop, &[]).is_none());

        let hidden = announcement(&laptop, VisibilityMode::TrustedOnly).unwrap();
        let txt: HashMap<String, String> = hidden.txt.into_iter().collect();
        assert!(interpret_service(&txt, [ip], 9000, &phone, &[]).is_none());
        let known = interpret_service(&txt, [ip], 9000, &phone, &[trusted(&laptop)]).unwrap();
        assert_eq!(known.name, "ThinkPad");

        assert!(announcement(&laptop, VisibilityMode::Off).is_none());
    }

    #[test]
    fn everyone_announcement_round_trips_and_shows_name_and_id() {
        let laptop = device("Kartikay's ThinkPad");
        let announcement = Announcement::of(&laptop, VisibilityMode::Everyone, 7).unwrap();
        let txt = txt_map(&announcement);
        assert_eq!(txt["vis"], "everyone");
        assert_eq!(txt["id"], hex::encode(laptop.device_id().0));
        assert_eq!(txt["type"], "laptop");
        assert_eq!(Announcement::parse(&txt), Some(announcement.clone()));
        assert!(!announcement.instance_name().contains("ThinkPad"));

        let seen = announcement
            .interpret(addr(), device("me").identity.public_key(), &[], 7)
            .unwrap();
        assert_eq!(seen.name, "Kartikay's ThinkPad");
        assert_eq!(seen.device_id, laptop.device_id());
        assert!(seen.trusted.is_none());
    }

    #[test]
    fn trusted_only_reveals_nothing_to_strangers() {
        let laptop = device("Kartikay's ThinkPad");
        let announcement = Announcement::of(&laptop, VisibilityMode::TrustedOnly, 7).unwrap();
        let txt = txt_map(&announcement);
        assert!(!txt.contains_key("name") && !txt.contains_key("id"));
        let everything: String =
            txt.values().cloned().collect::<Vec<_>>().join(" ") + &announcement.instance_name();
        assert!(!everything.contains("ThinkPad"));
        assert!(!everything.contains(&hex::encode(laptop.device_id().0)));

        let stranger = device("stranger");
        let parsed = Announcement::parse(&txt).unwrap();
        assert_eq!(
            parsed.interpret(addr(), stranger.identity.public_key(), &[], 7),
            None,
            "a device that doesn't trust it can't recognize it"
        );
    }

    #[test]
    fn trusted_only_is_recognized_by_devices_that_trust_it() {
        let laptop = device("ThinkPad");
        let phone = device("Pixel");
        let announcement = Announcement::of(&laptop, VisibilityMode::TrustedOnly, 100).unwrap();
        let parsed = Announcement::parse(&txt_map(&announcement)).unwrap();

        let seen = parsed
            .interpret(
                addr(),
                phone.identity.public_key(),
                &[trusted(&laptop)],
                100,
            )
            .unwrap();
        assert_eq!(seen.device_id, laptop.device_id());
        assert_eq!(seen.name, "ThinkPad", "the name comes from our trust store");
        assert!(seen.trusted.is_some());

        // One window of clock difference is tolerated; more is not.
        assert!(parsed
            .interpret(
                addr(),
                phone.identity.public_key(),
                &[trusted(&laptop)],
                101
            )
            .is_some());
        assert!(parsed
            .interpret(
                addr(),
                phone.identity.public_key(),
                &[trusted(&laptop)],
                103
            )
            .is_none());
    }

    #[test]
    fn rotating_id_changes_every_window_and_per_device() {
        let key = *device("a").identity.public_key();
        assert_eq!(rotating_id(&key, 5), rotating_id(&key, 5));
        assert_ne!(rotating_id(&key, 5), rotating_id(&key, 6));
        assert_ne!(
            rotating_id(&key, 5),
            rotating_id(device("b").identity.public_key(), 5)
        );
    }

    #[test]
    fn off_announces_nothing_and_own_announcements_are_hidden() {
        let me = device("me");
        assert_eq!(Announcement::of(&me, VisibilityMode::Off, 1), None);
        for mode in [VisibilityMode::Everyone, VisibilityMode::TrustedOnly] {
            let own = Announcement::of(&me, mode, 1).unwrap();
            assert_eq!(
                own.interpret(addr(), me.identity.public_key(), &[], 1),
                None
            );
        }
    }

    #[test]
    fn malformed_or_foreign_announcements_are_ignored() {
        let good = txt_map(&Announcement::of(&device("x"), VisibilityMode::Everyone, 1).unwrap());
        for (key, value) in [
            ("v", "2"),
            ("vis", "public"),
            ("id", "not-hex"),
            ("type", "toaster"),
        ] {
            let mut bad = good.clone();
            bad.insert(key.into(), value.into());
            assert_eq!(Announcement::parse(&bad), None, "{key}={value}");
        }
        let mut no_id = good.clone();
        no_id.remove("id");
        assert_eq!(Announcement::parse(&no_id), None);
    }

    #[test]
    fn addresses_prefer_routable_ipv4() {
        let ips = [
            "127.0.0.1".parse().unwrap(),
            "::1".parse().unwrap(),
            "fe80::1".parse().unwrap(),
            "169.254.3.4".parse().unwrap(),
            "2001:db8::5".parse().unwrap(),
            "192.168.1.20".parse().unwrap(),
        ];
        let sorted: Vec<String> = socket_addrs(ips.into_iter(), 9000)
            .iter()
            .map(|a| a.to_string())
            .collect();
        assert_eq!(
            sorted,
            [
                "192.168.1.20:9000",
                "[2001:db8::5]:9000",
                "169.254.3.4:9000",
                "[fe80::1]:9000",
                "127.0.0.1:9000",
                "[::1]:9000"
            ]
        );
    }

    fn nearby(name: &str, id_byte: u8) -> NearbyDevice {
        NearbyDevice {
            device_id: DeviceId([id_byte; 8]),
            name: name.into(),
            device_type: Some(DeviceType::Laptop),
            addresses: addr(),
            trusted: None,
            visibility: VisibilityMode::Everyone,
        }
    }

    #[test]
    fn queries_match_names_and_ids() {
        let devices = [
            nearby("ThinkPad", 1),
            nearby("Pixel", 2),
            nearby("pixel", 3),
        ];
        assert_eq!(
            match_query(&devices, "thinkpad").unwrap().device_id,
            DeviceId([1; 8])
        );
        assert_eq!(
            match_query(&devices, "0202-0202-0202-0202").unwrap().name,
            "Pixel"
        );
        assert_eq!(
            match_query(&devices, "0303030303030303").unwrap().name,
            "pixel"
        );
        assert!(matches!(
            match_query(&devices, "Pixel"),
            Err(DiscoveryError::Ambiguous { matches, .. }) if matches.len() == 2
        ));
        assert!(matches!(
            match_query(&devices, "iPhone"),
            Err(DiscoveryError::NotFound(_))
        ));
    }
}
