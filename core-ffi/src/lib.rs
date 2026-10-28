//! beam-core for Kotlin, through UniFFI.
//!
//! A [`BeamNode`] is one device: it receives (and asks the app about offers
//! through [`BeamListener`]), announces itself and watches for other devices
//! with mDNS (its own, or the platform's: see [`Discovery`]), sends files,
//! and pairs. It owns a tokio runtime, so calls from
//! Kotlin never block on networking: long operations are `suspend` functions
//! (`send`, `pair`), and everything else is reported as listener events.
//!
//! Files to send arrive as file descriptors (Android content URIs opened with
//! `ContentResolver`) or paths. A descriptor must be a seekable regular file,
//! because chunks are read at arbitrary offsets by parallel streams; the app
//! copies anything else (a pipe from a cloud provider) to its cache first.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use beam_core::discovery::{
    announcement, first_reachable, interpret_service, next_announcement_change,
};
use beam_core::{
    pair_with, send_sources, Advertiser, Browser, Decision, DeviceId, DeviceType, DiscoveryEvent,
    Identity, LocalDevice, PairingCode, PairingWindow, PeerInfo, Progress, ReceiveHandler,
    ReceiveOutcome, Receiver, ReceiverConfig, ReceiverHandle, SendEvent, SendOptions, SendOutcome,
    SendSource, TransferError, TrustStore,
};
use tokio::runtime::Runtime;
use tokio::sync::{oneshot, watch};
use uuid::Uuid;

uniffi::setup_scaffolding!();

/// How long an incoming offer waits for the user.
const OFFER_TIMEOUT: Duration = Duration::from_secs(60);

/// The field isn't called `message`: in Kotlin that would clash with
/// `Throwable.message`.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum BeamError {
    #[error("{reason}")]
    Failed { reason: String },
}

fn failed(e: impl std::fmt::Display) -> BeamError {
    BeamError::Failed {
        reason: e.to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Visibility {
    Everyone,
    TrustedOnly,
    Off,
}

impl From<Visibility> for beam_core::VisibilityMode {
    fn from(v: Visibility) -> Self {
        match v {
            Visibility::Everyone => beam_core::VisibilityMode::Everyone,
            Visibility::TrustedOnly => beam_core::VisibilityMode::TrustedOnly,
            Visibility::Off => beam_core::VisibilityMode::Off,
        }
    }
}

impl From<beam_core::VisibilityMode> for Visibility {
    fn from(v: beam_core::VisibilityMode) -> Self {
        match v {
            beam_core::VisibilityMode::Everyone => Visibility::Everyone,
            beam_core::VisibilityMode::TrustedOnly => Visibility::TrustedOnly,
            beam_core::VisibilityMode::Off => Visibility::Off,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DeviceKind {
    Phone,
    Laptop,
    Unknown,
}

impl From<Option<DeviceType>> for DeviceKind {
    fn from(kind: Option<DeviceType>) -> Self {
        match kind {
            Some(DeviceType::Phone) => DeviceKind::Phone,
            Some(DeviceType::Laptop) => DeviceKind::Laptop,
            None => DeviceKind::Unknown,
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct NodeConfig {
    pub device_name: String,
    pub device_kind: DeviceKind,
    /// From [`generate_identity_secret`], kept by the app in a keystore.
    pub identity_secret: Vec<u8>,
    /// Where the trust store lives.
    pub data_dir: String,
    /// Where incoming files are written (and kept for resume) until done.
    pub incoming_dir: String,
    pub visibility: Visibility,
    /// TCP port to listen on; 0 picks a free one.
    pub port: u16,
    /// Address to listen on; `None` means every interface.
    pub listen_address: Option<String>,
    pub discovery: Discovery,
}

/// Who does mDNS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Discovery {
    /// The node announces and browses itself (the `mdns-sd` crate).
    BuiltIn,
    /// The app does (Android's `NsdManager`): it registers what
    /// `advertisement_changed` says, and reports what it resolves with
    /// `service_found` / `service_lost`.
    Platform,
    /// No discovery; send to addresses only.
    Off,
}

/// A service for the app to register with the platform's mDNS.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ServiceAdvert {
    /// Instance name; register it under `_beam._tcp`.
    pub instance_name: String,
    pub port: u16,
    pub txt: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NearbyDevice {
    pub device_id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub address: Option<String>,
    pub trusted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OfferFile {
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct IncomingOffer {
    /// Answer it with [`BeamNode::respond_offer`].
    pub offer_id: String,
    pub from_name: String,
    pub from_id: String,
    pub trusted: bool,
    pub verification_code: String,
    pub files: Vec<OfferFile>,
    pub total_bytes: u64,
    pub timeout_secs: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Direction {
    Send,
    Receive,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TransferInfo {
    pub transfer_id: String,
    pub direction: Direction,
    pub peer_name: String,
    pub file_names: Vec<String>,
    pub file_count: u32,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TransferResult {
    Completed,
    Declined,
    Cancelled,
    Failed,
    /// The connection dropped; the sender may still reconnect and resume.
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TrustedDevice {
    pub device_id: String,
    pub name: String,
    pub auto_accept: bool,
}

impl From<beam_core::TrustedDevice> for TrustedDevice {
    fn from(d: beam_core::TrustedDevice) -> Self {
        TrustedDevice {
            device_id: d.device_id.to_string(),
            name: d.name,
            auto_accept: d.auto_accept,
        }
    }
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum FileSource {
    /// A file descriptor the node takes ownership of (and closes). Kotlin:
    /// `ParcelFileDescriptor.detachFd()`.
    Fd {
        fd: i32,
    },
    Path {
        path: String,
    },
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct OutgoingFile {
    /// The name the receiver sees (for a content URI, its display name).
    pub name: String,
    pub modified_unix: i64,
    pub source: FileSource,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PairingCodeInfo {
    pub device_id: String,
    pub name: String,
    pub addresses: Vec<String>,
}

/// What the app hears from the node. Called on background threads.
#[uniffi::export(with_foreign)]
pub trait BeamListener: Send + Sync {
    fn device_found(&self, device: NearbyDevice);
    fn device_lost(&self, device_id: String);
    /// Someone wants to send: ask the user, then call `respond_offer`.
    fn offer_received(&self, offer: IncomingOffer);
    /// The offer was answered, or timed out: dismiss any prompt for it.
    fn offer_closed(&self, offer_id: String);
    fn transfer_started(&self, transfer: TransferInfo);
    /// Something to show while it runs: "Waiting for them to accept…",
    /// the verification code, "Reconnecting…".
    fn transfer_status(
        &self,
        transfer_id: String,
        status: String,
        verification_code: Option<String>,
    );
    fn transfer_progress(&self, transfer_id: String, done_bytes: u64, total_bytes: u64);
    fn transfer_finished(
        &self,
        transfer_id: String,
        result: TransferResult,
        message: Option<String>,
        saved_files: Vec<String>,
    );
    /// A device that scanned our pairing code is now trusted.
    fn paired(&self, device: TrustedDevice);
    /// With [`Discovery::Platform`]: what to register now (replacing any
    /// earlier one), or `None` to unregister.
    fn advertisement_changed(&self, advert: Option<ServiceAdvert>);
}

/// A new device key, for the app to keep in its keystore.
#[uniffi::export]
pub fn generate_identity_secret() -> Vec<u8> {
    Identity::generate().export_secret().to_vec()
}

/// The device id (fingerprint) of a stored key.
#[uniffi::export]
pub fn device_id_for_secret(secret: Vec<u8>) -> Result<String, BeamError> {
    Identity::import_secret(&secret)
        .map(|identity| identity.device_id().to_string())
        .ok_or_else(|| failed("not a Beam device key"))
}

/// Reads a scanned `beam://pair` QR code (without contacting anyone).
#[uniffi::export]
pub fn parse_pairing_code(code: String) -> Result<PairingCodeInfo, BeamError> {
    let code = PairingCode::parse(code.trim()).map_err(failed)?;
    Ok(PairingCodeInfo {
        device_id: code.device_id.to_string(),
        name: code.name,
        addresses: code.addresses.iter().map(|a| a.to_string()).collect(),
    })
}

#[derive(uniffi::Object)]
pub struct BeamNode {
    runtime: Runtime,
    inner: Arc<Inner>,
}

struct Inner {
    device: LocalDevice,
    trust: TrustStore,
    visibility: beam_core::Visibility,
    pairing: PairingWindow,
    incoming_dir: PathBuf,
    listen_address: String,
    port: u16,
    discovery: Discovery,
    listener: Arc<dyn BeamListener>,
    streams: Mutex<usize>,
    offers: Mutex<HashMap<String, PendingOffer>>,
    sends: Mutex<HashMap<String, watch::Sender<bool>>>,
    nearby: Mutex<HashMap<DeviceId, beam_core::NearbyDevice>>,
    /// Platform service name → the device it announced.
    services: Mutex<HashMap<String, DeviceId>>,
    running: Mutex<Option<Running>>,
}

/// An offer waiting for `respond_offer`, with who it's from (for "trust").
struct PendingOffer {
    reply: oneshot::Sender<Decision>,
    from_name: String,
    from_key: [u8; 32],
}

struct Running {
    receiver: ReceiverHandle,
    port: u16,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    _advertiser: Option<Advertiser>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

#[uniffi::export]
impl BeamNode {
    #[uniffi::constructor]
    pub fn new(
        config: NodeConfig,
        listener: Arc<dyn BeamListener>,
    ) -> Result<Arc<Self>, BeamError> {
        let identity = Identity::import_secret(&config.identity_secret)
            .ok_or_else(|| failed("not a Beam device key"))?;
        let data_dir = PathBuf::from(&config.data_dir);
        std::fs::create_dir_all(&data_dir).map_err(failed)?;
        let trust = TrustStore::open(&data_dir.join("trusted_devices.json")).map_err(failed)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("beam")
            .enable_all()
            .build()
            .map_err(failed)?;
        Ok(Arc::new(BeamNode {
            runtime,
            inner: Arc::new(Inner {
                device: LocalDevice {
                    identity,
                    name: config.device_name,
                    device_type: match config.device_kind {
                        DeviceKind::Laptop => DeviceType::Laptop,
                        _ => DeviceType::Phone,
                    },
                },
                trust,
                visibility: beam_core::Visibility::new(config.visibility.into()),
                pairing: PairingWindow::default(),
                incoming_dir: PathBuf::from(config.incoming_dir),
                listen_address: config.listen_address.unwrap_or_else(|| "0.0.0.0".into()),
                port: config.port,
                discovery: config.discovery,
                listener,
                streams: Mutex::new(4),
                offers: Mutex::default(),
                sends: Mutex::default(),
                nearby: Mutex::default(),
                services: Mutex::default(),
                running: Mutex::default(),
            }),
        }))
    }

    pub fn device_id(&self) -> String {
        self.inner.device.device_id().to_string()
    }

    pub fn device_name(&self) -> String {
        self.inner.device.name.clone()
    }

    /// Starts receiving, announcing and browsing. Returns the TCP port.
    /// Calling it again while running does nothing.
    pub fn start(&self) -> Result<u16, BeamError> {
        if let Some(running) = lock(&self.inner.running).as_ref() {
            return Ok(running.port);
        }
        let inner = self.inner.clone();
        let running = self.runtime.block_on(async move { inner.start().await })?;
        let port = running.port;
        *lock(&self.inner.running) = Some(running);
        Ok(port)
    }

    /// Stops receiving and announcing. Transfers in progress are cut (a
    /// sender will try to resume later).
    pub fn stop(&self) {
        if let Some(running) = lock(&self.inner.running).take() {
            for task in running.tasks {
                task.abort();
            }
            if self.inner.discovery == Discovery::Platform {
                self.inner.listener.advertisement_changed(None);
            }
        }
        lock(&self.inner.nearby).clear();
        lock(&self.inner.services).clear();
    }

    /// With [`Discovery::Platform`]: the platform resolved a `_beam._tcp`
    /// service. Reports `device_found` if it's a device to show.
    pub fn service_found(
        &self,
        service_name: String,
        txt: HashMap<String, String>,
        addresses: Vec<String>,
        port: u16,
    ) {
        let ips = addresses.iter().filter_map(|a| a.parse().ok());
        let trusted = self.inner.trust.list().unwrap_or_default();
        let Some(device) = interpret_service(&txt, ips, port, &self.inner.device, &trusted) else {
            return;
        };
        lock(&self.inner.services).insert(service_name, device.device_id);
        self.inner.found(device);
    }

    /// With [`Discovery::Platform`]: the service went away.
    pub fn service_lost(&self, service_name: String) {
        let Some(id) = lock(&self.inner.services).remove(&service_name) else {
            return;
        };
        // The same device may still be announced under another name.
        if lock(&self.inner.services)
            .values()
            .any(|other| *other == id)
        {
            return;
        }
        self.inner.lost(id);
    }

    pub fn visibility(&self) -> Visibility {
        self.inner.visibility.mode().into()
    }

    pub fn set_visibility(&self, visibility: Visibility) {
        self.inner.visibility.set(visibility.into());
    }

    /// Parallel connections for sending (1–16).
    pub fn set_streams(&self, streams: u32) {
        *lock(&self.inner.streams) = (streams as usize).clamp(1, 16);
    }

    /// Answers `offer_received`. `trust` also trusts the sender, with
    /// auto-accept, for next time.
    pub fn respond_offer(
        &self,
        offer_id: String,
        accept: bool,
        trust: bool,
    ) -> Result<(), BeamError> {
        let pending = lock(&self.inner.offers)
            .remove(&offer_id)
            .ok_or_else(|| failed("this offer has expired"))?;
        if accept && trust {
            self.inner
                .trust
                .upsert(beam_core::TrustedDevice::new(
                    pending.from_name,
                    pending.from_key,
                    true,
                ))
                .map_err(failed)?;
        }
        let _ = pending.reply.send(if accept {
            Decision::Accept
        } else {
            Decision::Reject("declined".into())
        });
        Ok(())
    }

    /// Sends files to `target`: a device id from `device_found`, or an
    /// `ip:port` address. Pick `transfer_id` yourself (a UUID); events and
    /// `cancel` use it. Returns when the transfer ends.
    pub async fn send(
        &self,
        transfer_id: String,
        target: String,
        files: Vec<OutgoingFile>,
    ) -> Result<TransferResult, BeamError> {
        let inner = self.inner.clone();
        self.runtime
            .spawn(async move { inner.send(transfer_id, target, files).await })
            .await
            .map_err(failed)?
    }

    /// Cancels a transfer in either direction.
    pub fn cancel(&self, transfer_id: String) {
        if let Some(cancel) = lock(&self.inner.sends).get(&transfer_id) {
            let _ = cancel.send(true);
            return;
        }
        if let (Ok(id), Some(running)) = (
            transfer_id.parse::<Uuid>(),
            lock(&self.inner.running).as_ref(),
        ) {
            running.receiver.cancel(id);
        }
    }

    pub fn trusted_devices(&self) -> Vec<TrustedDevice> {
        self.inner
            .trust
            .list()
            .unwrap_or_default()
            .into_iter()
            .map(TrustedDevice::from)
            .collect()
    }

    pub fn set_auto_accept(&self, device_id: String, auto_accept: bool) -> Result<(), BeamError> {
        let id: DeviceId = device_id.parse().map_err(failed)?;
        let mut device = self
            .inner
            .trust
            .get(&id)
            .map_err(failed)?
            .ok_or_else(|| failed("not a trusted device"))?;
        device.auto_accept = auto_accept;
        self.inner.trust.upsert(device).map_err(failed)
    }

    pub fn remove_trusted(&self, device_id: String) -> Result<(), BeamError> {
        let id: DeviceId = device_id.parse().map_err(failed)?;
        self.inner.trust.remove(&id).map(|_| ()).map_err(failed)
    }

    /// Pairs with the device whose QR code was scanned: both sides end up
    /// trusting each other.
    pub async fn pair(&self, code: String) -> Result<TrustedDevice, BeamError> {
        let inner = self.inner.clone();
        self.runtime
            .spawn(async move {
                let code = PairingCode::parse(code.trim()).map_err(failed)?;
                pair_with(&code, &inner.device, &inner.trust)
                    .await
                    .map(TrustedDevice::from)
                    .map_err(failed)
            })
            .await
            .map_err(failed)?
    }
}

impl Inner {
    async fn start(self: Arc<Self>) -> Result<Running, BeamError> {
        std::fs::create_dir_all(&self.incoming_dir).map_err(failed)?;
        let config = ReceiverConfig {
            device: self.device.clone(),
            download_dir: self.incoming_dir.clone(),
            trust: self.trust.clone(),
            visibility: self.visibility.clone(),
            pairing: self.pairing.clone(),
        };
        let receiver = Receiver::bind((self.listen_address.as_str(), self.port), config)
            .await
            .map_err(|e| failed(format!("could not start receiving: {e}")))?;
        let port = receiver.local_addr().map_err(failed)?.port();
        let handle = receiver.handle();
        let receive_task = tokio::spawn({
            let handler = NodeHandler(self.clone());
            async move {
                let _ = receiver.run(handler).await;
            }
        });

        let mut tasks = vec![receive_task];
        let advertiser = match self.discovery {
            Discovery::Off => None,
            Discovery::Platform => {
                tasks.push(tokio::spawn(self.clone().advertise_through_platform(port)));
                None
            }
            Discovery::BuiltIn => {
                let advertiser =
                    Advertiser::start(self.device.clone(), port, self.visibility.clone())
                        .map_err(failed)?;
                let mut browser =
                    Browser::start(&self.device, self.trust.clone()).map_err(failed)?;
                let inner = self.clone();
                tasks.push(tokio::spawn(async move {
                    while let Some(event) = browser.next().await {
                        match event {
                            DiscoveryEvent::Found(device) => inner.found(device),
                            DiscoveryEvent::Lost(id) => inner.lost(id),
                        }
                    }
                }));
                Some(advertiser)
            }
        };
        Ok(Running {
            receiver: handle,
            port,
            tasks,
            _advertiser: advertiser,
        })
    }

    /// Tells the app what to register whenever the announcement changes:
    /// visibility switched, "everyone" ran out, or the rotating id moved on.
    async fn advertise_through_platform(self: Arc<Self>, port: u16) {
        let mut changes = self.visibility.subscribe();
        let mut current: Option<ServiceAdvert> = None;
        let mut first = true;
        loop {
            let wanted =
                announcement(&self.device, self.visibility.mode()).map(|a| ServiceAdvert {
                    instance_name: a.instance_name,
                    port,
                    txt: a.txt.into_iter().collect(),
                });
            if first || wanted != current {
                self.listener.advertisement_changed(wanted.clone());
                current = wanted;
                first = false;
            }
            tokio::select! {
                _ = changes.changed() => {}
                _ = tokio::time::sleep_until(next_announcement_change(&self.visibility)) => {}
            }
        }
    }

    fn found(&self, device: beam_core::NearbyDevice) {
        lock(&self.nearby).insert(device.device_id, device.clone());
        self.listener.device_found(NearbyDevice {
            device_id: device.device_id.to_string(),
            name: device.name,
            kind: device.device_type.into(),
            address: device.addresses.first().map(|a| a.to_string()),
            trusted: device.trusted.is_some(),
        });
    }

    fn lost(&self, id: DeviceId) {
        lock(&self.nearby).remove(&id);
        self.listener.device_lost(id.to_string());
    }

    async fn send(
        self: Arc<Self>,
        transfer_id: String,
        target: String,
        files: Vec<OutgoingFile>,
    ) -> Result<TransferResult, BeamError> {
        let file_names: Vec<String> = files.iter().map(|f| f.name.clone()).collect();
        // Open every file before giving up on any, so each descriptor the
        // app handed over is owned (and closed) even when one fails.
        let opened: Vec<_> = files.into_iter().map(open_source).collect();
        let sources = opened.into_iter().collect::<Result<Vec<_>, _>>()?;
        let (addr, expect_device, peer_name) = self.resolve(&target).await?;
        let (cancel_tx, cancel_rx) = watch::channel(false);
        lock(&self.sends).insert(transfer_id.clone(), cancel_tx);

        self.listener.transfer_started(TransferInfo {
            transfer_id: transfer_id.clone(),
            direction: Direction::Send,
            peer_name,
            file_count: file_names.len() as u32,
            file_names: file_names.into_iter().take(5).collect(),
            total_bytes: 0,
        });
        self.listener
            .transfer_status(transfer_id.clone(), "Connecting…".into(), None);
        let options = SendOptions {
            streams: *lock(&self.streams),
            expect_device,
            cancel: Some(cancel_rx),
            ..SendOptions::default()
        };
        let listener = self.listener.clone();
        let id = transfer_id.clone();
        let result = send_sources(
            addr,
            &self.device,
            &self.trust,
            sources,
            &options,
            |event| match event {
                SendEvent::Connected(peer) => listener.transfer_status(
                    id.clone(),
                    format!("Waiting for {} to accept…", peer.name()),
                    Some(peer.verification_code.clone()),
                ),
                SendEvent::Accepted { .. } => {
                    listener.transfer_status(id.clone(), "Sending…".into(), None)
                }
                SendEvent::Progress(Progress {
                    bytes_done,
                    bytes_total,
                    ..
                }) => listener.transfer_progress(id.clone(), bytes_done, bytes_total),
                SendEvent::Reconnecting { attempt, .. } => listener.transfer_status(
                    id.clone(),
                    format!("Connection lost, reconnecting (attempt {attempt})…"),
                    None,
                ),
            },
        )
        .await;
        lock(&self.sends).remove(&transfer_id);

        let (outcome, message) = match result {
            Ok(SendOutcome::Completed(_)) => (TransferResult::Completed, None),
            Ok(SendOutcome::Rejected { reason }) => (TransferResult::Declined, Some(reason)),
            Err(TransferError::CancelledByUser) => (TransferResult::Cancelled, None),
            Err(TransferError::Cancelled(reason)) => (TransferResult::Cancelled, Some(reason)),
            Err(error) => (TransferResult::Failed, Some(error.to_string())),
        };
        self.listener
            .transfer_finished(transfer_id, outcome, message, Vec::new());
        Ok(outcome)
    }

    /// A device id from discovery, or a literal address.
    async fn resolve(
        &self,
        target: &str,
    ) -> Result<(std::net::SocketAddr, Option<DeviceId>, String), BeamError> {
        if let Ok(addr) = target.parse() {
            return Ok((addr, None, target.to_string()));
        }
        let id: DeviceId = target
            .parse()
            .map_err(|_| failed(format!("{target:?} is neither a device nor an address")))?;
        let device = lock(&self.nearby)
            .get(&id)
            .cloned()
            .ok_or_else(|| failed("that device is no longer on the network"))?;
        let addr = first_reachable(&device.addresses)
            .await
            .ok_or_else(|| failed(format!("\"{}\" didn't answer", device.name)))?;
        Ok((addr, Some(id), device.name))
    }
}

/// Turns an `OutgoingFile` into something the sender can read.
fn open_source(file: OutgoingFile) -> Result<SendSource, BeamError> {
    match file.source {
        FileSource::Path { path } => {
            let path = PathBuf::from(path);
            if file.name.is_empty() {
                Ok(SendSource::path(path))
            } else {
                let opened = std::fs::File::open(&path).map_err(failed)?;
                Ok(SendSource::open_file(opened, file.name, file.modified_unix))
            }
        }
        #[cfg(unix)]
        FileSource::Fd { fd } => {
            use std::os::fd::FromRawFd;
            if fd < 0 {
                return Err(failed("invalid file descriptor"));
            }
            // SAFETY: the app detached this descriptor and hands its
            // ownership to us; nothing else will use or close it.
            let opened = unsafe { std::fs::File::from_raw_fd(fd) };
            Ok(SendSource::open_file(opened, file.name, file.modified_unix))
        }
        #[cfg(not(unix))]
        FileSource::Fd { .. } => Err(failed("file descriptors are only supported on Android")),
    }
}

/// Bridges beam-core's receiver to the listener.
struct NodeHandler(Arc<Inner>);

impl ReceiveHandler for NodeHandler {
    fn decide(&self, offer: beam_core::IncomingOffer) -> beam_core::BoxFuture<Decision> {
        let inner = self.0.clone();
        Box::pin(async move {
            let offer_id = offer.transfer_id.to_string();
            let (reply, answer) = oneshot::channel();
            lock(&inner.offers).insert(
                offer_id.clone(),
                PendingOffer {
                    reply,
                    from_name: offer.from.name().to_string(),
                    from_key: offer.from.public_key,
                },
            );
            inner.listener.offer_received(IncomingOffer {
                offer_id: offer_id.clone(),
                from_name: offer.from.name().to_string(),
                from_id: offer.from.device_id().to_string(),
                trusted: offer.from.is_trusted(),
                verification_code: offer.from.verification_code.clone(),
                files: offer
                    .files
                    .iter()
                    .map(|f| OfferFile {
                        name: f.name.clone(),
                        size: f.size,
                    })
                    .collect(),
                total_bytes: offer.total_size,
                timeout_secs: OFFER_TIMEOUT.as_secs() as u32,
            });
            let decision = match tokio::time::timeout(OFFER_TIMEOUT, answer).await {
                Ok(Ok(decision)) => decision,
                _ => Decision::Reject("no answer".into()),
            };
            lock(&inner.offers).remove(&offer_id);
            inner.listener.offer_closed(offer_id);
            decision
        })
    }

    fn accepted(&self, offer: &beam_core::IncomingOffer, _resumed: bool) {
        self.0.listener.transfer_started(TransferInfo {
            transfer_id: offer.transfer_id.to_string(),
            direction: Direction::Receive,
            peer_name: offer.from.name().to_string(),
            file_names: offer.files.iter().take(5).map(|f| f.name.clone()).collect(),
            file_count: offer.files.len() as u32,
            total_bytes: offer.total_size,
        });
        self.0.listener.transfer_status(
            offer.transfer_id.to_string(),
            "Receiving…".into(),
            Some(offer.from.verification_code.clone()),
        );
    }

    fn progress(&self, transfer_id: Uuid, progress: Progress) {
        self.0.listener.transfer_progress(
            transfer_id.to_string(),
            progress.bytes_done,
            progress.bytes_total,
        );
    }

    fn finished(&self, transfer_id: Uuid, result: &Result<ReceiveOutcome, TransferError>) {
        let (outcome, message, saved) = match result {
            Ok(ReceiveOutcome::Completed { files, .. }) => (
                TransferResult::Completed,
                None,
                files.iter().map(|f| f.path.display().to_string()).collect(),
            ),
            Ok(ReceiveOutcome::Rejected { reason, .. }) => {
                (TransferResult::Declined, Some(reason.clone()), Vec::new())
            }
            Err(e) if e.is_retryable() || matches!(e, TransferError::Superseded) => {
                (TransferResult::Interrupted, Some(e.to_string()), Vec::new())
            }
            Err(TransferError::CancelledByUser) => (TransferResult::Cancelled, None, Vec::new()),
            Err(TransferError::Cancelled(reason)) => {
                (TransferResult::Cancelled, Some(reason.clone()), Vec::new())
            }
            Err(e) => (TransferResult::Failed, Some(e.to_string()), Vec::new()),
        };
        self.0
            .listener
            .transfer_finished(transfer_id.to_string(), outcome, message, saved);
    }

    fn paired(&self, device: &PeerInfo) {
        self.0.listener.paired(TrustedDevice {
            device_id: device.device_id().to_string(),
            name: device.name().to_string(),
            auto_accept: true,
        });
    }
}
