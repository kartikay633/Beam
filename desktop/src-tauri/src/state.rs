//! Everything the app knows, and the glue between beam-core and the UI.
//!
//! The UI never asks for pieces of state: whenever something changes, the
//! backend sends a complete [`Snapshot`] (event `snapshot`). Progress updates
//! are throttled so a fast transfer doesn't flood the WebView.

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use beam_core::discovery::first_reachable;
use beam_core::{
    send_to, Advertiser, BoxFuture, Browser, Decision, DeviceId, DeviceType, DiscoveryEvent,
    IncomingOffer, LocalDevice, NearbyDevice, PairingWindow, Progress, ReceiveHandler,
    ReceiveOutcome, Receiver, ReceiverConfig, ReceiverHandle, SendEvent, SendOptions, SendOutcome,
    TransferError, TrustStore, TrustedDevice, Visibility, VisibilityMode,
};
use serde::Serialize;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::{oneshot, watch};
use uuid::Uuid;

use crate::settings::Settings;

/// How long the incoming-offer popup waits for an answer.
pub const OFFER_TIMEOUT: Duration = Duration::from_secs(60);
/// The receiver's preferred port; another free port is used if it's taken.
const PREFERRED_PORT: u16 = 9000;
const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(150);
/// Finished transfers kept in the list.
const MAX_FINISHED: usize = 50;

pub struct AppState {
    pub app: AppHandle,
    config_dir: PathBuf,
    pub device: LocalDevice,
    pub trust: TrustStore,
    pub visibility: Visibility,
    /// The pairing code on the Pair page, if it's open.
    pub pairing: PairingWindow,
    settings: Mutex<Settings>,
    ui: Mutex<UiState>,
    receiver: Mutex<Option<RunningReceiver>>,
    advertiser: Mutex<Option<Advertiser>>,
    last_emit: Mutex<Instant>,
}

struct RunningReceiver {
    handle: ReceiverHandle,
    port: u16,
    task: JoinHandle<()>,
}

#[derive(Default)]
struct UiState {
    nearby: BTreeMap<DeviceId, NearbyDevice>,
    /// Newest first.
    transfers: Vec<Transfer>,
    offers: HashMap<Uuid, PendingOffer>,
    /// Files waiting for the user to pick a device (dropped, chosen, or
    /// handed over by Explorer's Send To).
    staged: Vec<PathBuf>,
}

struct Transfer {
    view: TransferView,
    rate: RateMeter,
    /// Sending: set to cancel. Receiving uses the receiver handle instead.
    cancel: Option<watch::Sender<bool>>,
}

struct PendingOffer {
    view: OfferView,
    peer_name: String,
    peer_key: [u8; 32],
    reply: Option<oneshot::Sender<Decision>>,
}

// ---------------------------------------------------------------- views

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub me: MeView,
    pub visibility: VisibilityView,
    pub nearby: Vec<DeviceView>,
    pub transfers: Vec<TransferView>,
    pub trusted: Vec<TrustedView>,
    pub staged: Vec<StagedFile>,
    pub settings: Settings,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeView {
    pub name: String,
    pub device_id: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityView {
    pub mode: &'static str,
    /// When "everyone" ends, in Unix milliseconds.
    pub everyone_until_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    pub device_id: String,
    pub name: String,
    pub kind: &'static str,
    pub address: Option<String>,
    pub trusted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferState {
    /// Connecting, or waiting for the other side to accept.
    Waiting,
    Active,
    Reconnecting,
    Done,
    Declined,
    Failed,
    Cancelled,
}

impl TransferState {
    fn is_finished(self) -> bool {
        matches!(
            self,
            TransferState::Done
                | TransferState::Declined
                | TransferState::Failed
                | TransferState::Cancelled
        )
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferView {
    pub id: String,
    pub direction: &'static str,
    pub peer_name: String,
    pub file_names: Vec<String>,
    pub file_count: usize,
    pub total_bytes: u64,
    pub done_bytes: u64,
    pub state: TransferState,
    /// Why it failed or was declined, or what it's doing right now.
    pub detail: Option<String>,
    pub speed_bps: f64,
    pub eta_secs: Option<f64>,
    pub verification_code: Option<String>,
    /// Where received files went.
    pub folder: Option<String>,
    pub started_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedView {
    pub device_id: String,
    pub name: String,
    pub auto_accept: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedFile {
    pub path: String,
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferView {
    pub id: String,
    pub from_name: String,
    pub from_id: String,
    pub trusted: bool,
    pub verification_code: String,
    pub files: Vec<OfferFile>,
    pub total_bytes: u64,
    /// When the popup gives up, in Unix milliseconds.
    pub expires_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferFile {
    pub name: String,
    pub size: u64,
}

/// Smoothed transfer speed (exponential moving average over ~0.5 s samples).
#[derive(Debug, Default)]
struct RateMeter {
    last: Option<(Instant, u64)>,
    bytes_per_sec: f64,
}

impl RateMeter {
    fn update(&mut self, done: u64) {
        let now = Instant::now();
        match self.last {
            None => self.last = Some((now, done)),
            Some((at, bytes)) => {
                let elapsed = now.duration_since(at).as_secs_f64();
                if elapsed >= 0.5 {
                    let sample = done.saturating_sub(bytes) as f64 / elapsed;
                    self.bytes_per_sec = if self.bytes_per_sec == 0.0 {
                        sample
                    } else {
                        0.6 * self.bytes_per_sec + 0.4 * sample
                    };
                    self.last = Some((now, done));
                }
            }
        }
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn device_kind(kind: Option<DeviceType>) -> &'static str {
    match kind {
        Some(DeviceType::Phone) => "phone",
        Some(DeviceType::Laptop) => "laptop",
        None => "device",
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

// ---------------------------------------------------------------- state

impl AppState {
    pub fn new(app: AppHandle, config_dir: PathBuf) -> Result<Arc<AppState>, String> {
        let settings = Settings::load(&config_dir.join("settings.json"));
        let identity = crate::keystore::load_or_create(&config_dir)?;
        let trust = TrustStore::open(&config_dir.join("trusted_devices.json"))
            .map_err(|e| e.to_string())?;
        let mode = settings
            .visibility
            .parse()
            .unwrap_or(VisibilityMode::Everyone);
        Ok(Arc::new(AppState {
            app,
            device: LocalDevice {
                identity,
                name: settings.device_name.clone(),
                device_type: DeviceType::Laptop,
            },
            trust,
            visibility: Visibility::new(mode),
            pairing: PairingWindow::default(),
            settings: Mutex::new(settings),
            config_dir,
            ui: Mutex::default(),
            receiver: Mutex::default(),
            advertiser: Mutex::default(),
            last_emit: Mutex::new(Instant::now()),
        }))
    }

    /// Starts receiving, announcing and browsing.
    pub async fn start(self: &Arc<Self>) -> Result<(), String> {
        let port = self.start_receiver().await?;
        let advertiser = Advertiser::start(self.device.clone(), port, self.visibility.clone())
            .map_err(|e| e.to_string())?;
        *lock(&self.advertiser) = Some(advertiser);
        self.start_browser()?;
        self.watch_visibility_expiry();
        self.emit();
        Ok(())
    }

    async fn start_receiver(self: &Arc<Self>) -> Result<u16, String> {
        let config = ReceiverConfig {
            device: self.device.clone(),
            download_dir: self.settings().download_dir,
            trust: self.trust.clone(),
            visibility: self.visibility.clone(),
            pairing: self.pairing.clone(),
        };
        let previous_port = lock(&self.receiver).as_ref().map(|r| r.port);
        let receiver = match Receiver::bind(
            ("0.0.0.0", previous_port.unwrap_or(PREFERRED_PORT)),
            config.clone(),
        )
        .await
        {
            Ok(receiver) => receiver,
            Err(_) => Receiver::bind(("0.0.0.0", 0), config)
                .await
                .map_err(|e| format!("could not start receiving: {e}"))?,
        };
        let port = receiver.local_addr().map_err(|e| e.to_string())?.port();
        let handle = receiver.handle();
        let task = tauri::async_runtime::spawn({
            let handler = DesktopHandler(self.clone());
            async move {
                let _ = receiver.run(handler).await;
            }
        });
        if let Some(old) = lock(&self.receiver).replace(RunningReceiver { handle, port, task }) {
            old.task.abort();
        }
        Ok(port)
    }

    fn start_browser(self: &Arc<Self>) -> Result<(), String> {
        let mut browser =
            Browser::start(&self.device, self.trust.clone()).map_err(|e| e.to_string())?;
        let state = self.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(event) = browser.next().await {
                {
                    let mut ui = state.ui();
                    match event {
                        DiscoveryEvent::Found(device) => {
                            ui.nearby.insert(device.device_id, device);
                        }
                        DiscoveryEvent::Lost(id) => {
                            ui.nearby.remove(&id);
                        }
                    }
                }
                state.emit();
            }
        });
        Ok(())
    }

    /// Refreshes the UI and tray when "everyone" runs out.
    fn watch_visibility_expiry(self: &Arc<Self>) {
        let state = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut changes = state.visibility.subscribe();
            loop {
                let expiry = state.visibility.everyone_until();
                tokio::select! {
                    _ = changes.changed() => {}
                    _ = async {
                        match expiry {
                            Some(until) => tokio::time::sleep_until(until).await,
                            None => std::future::pending().await,
                        }
                    } => {
                        let _ = state.visibility.mode();
                        crate::tray::show_visibility(&state.app, state.visibility.mode());
                        state.emit();
                    }
                }
            }
        });
    }

    fn ui(&self) -> MutexGuard<'_, UiState> {
        lock(&self.ui)
    }

    pub fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    pub fn port(&self) -> u16 {
        lock(&self.receiver).as_ref().map_or(0, |r| r.port)
    }

    // ------------------------------------------------------------ snapshots

    pub fn snapshot(&self) -> Snapshot {
        let mode = self.visibility.mode();
        let everyone_until_ms = self.visibility.everyone_until().map(|until| {
            now_ms()
                + until
                    .saturating_duration_since(tokio::time::Instant::now())
                    .as_millis() as u64
        });
        let ui = self.ui();
        let mut nearby: Vec<DeviceView> = ui
            .nearby
            .values()
            .map(|d| DeviceView {
                device_id: d.device_id.to_string(),
                name: d.name.clone(),
                kind: device_kind(d.device_type),
                address: d.addresses.first().map(|a| a.to_string()),
                trusted: d.trusted.is_some(),
            })
            .collect();
        // Trusted devices first, then by name.
        nearby.sort_by(|a, b| {
            b.trusted
                .cmp(&a.trusted)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Snapshot {
            me: MeView {
                name: self.device.name.clone(),
                device_id: self.device.device_id().to_string(),
                port: self.port(),
            },
            visibility: VisibilityView {
                mode: mode.as_str(),
                everyone_until_ms,
            },
            nearby,
            transfers: ui.transfers.iter().map(|t| t.view.clone()).collect(),
            trusted: self
                .trust
                .list()
                .unwrap_or_default()
                .into_iter()
                .map(|t| TrustedView {
                    device_id: t.device_id.to_string(),
                    name: t.name,
                    auto_accept: t.auto_accept,
                })
                .collect(),
            staged: ui
                .staged
                .iter()
                .map(|p| StagedFile {
                    path: p.display().to_string(),
                    name: file_name(p),
                    size: std::fs::metadata(p).map_or(0, |m| m.len()),
                })
                .collect(),
            settings: self.settings(),
        }
    }

    /// Sends a fresh snapshot to every window.
    pub fn emit(&self) {
        *lock(&self.last_emit) = Instant::now();
        let _ = self.app.emit("snapshot", self.snapshot());
    }

    /// Like [`AppState::emit`], at most every 150 ms (progress updates).
    fn emit_throttled(&self) {
        let due = lock(&self.last_emit).elapsed() >= PROGRESS_EMIT_INTERVAL;
        if due {
            self.emit();
        }
    }

    fn update_transfer(&self, id: &str, change: impl FnOnce(&mut Transfer)) {
        let mut ui = self.ui();
        if let Some(transfer) = ui.transfers.iter_mut().find(|t| t.view.id == id) {
            change(transfer);
        }
    }

    fn add_transfer(&self, transfer: Transfer) {
        let mut ui = self.ui();
        ui.transfers.retain(|t| t.view.id != transfer.view.id);
        ui.transfers.insert(0, transfer);
        let mut finished = 0;
        ui.transfers.retain(|t| {
            if t.view.state.is_finished() {
                finished += 1;
                finished <= MAX_FINISHED
            } else {
                true
            }
        });
    }

    // ------------------------------------------------------------ staging

    pub fn stage(&self, paths: Vec<PathBuf>) {
        {
            let mut ui = self.ui();
            for path in paths {
                if path.is_file() && !ui.staged.contains(&path) {
                    ui.staged.push(path);
                }
            }
        }
        self.emit();
    }

    pub fn unstage(&self, path: Option<PathBuf>) {
        {
            let mut ui = self.ui();
            match path {
                Some(path) => ui.staged.retain(|p| *p != path),
                None => ui.staged.clear(),
            }
        }
        self.emit();
    }

    // ------------------------------------------------------------ sending

    /// Sends the staged files to `target` (a device id seen on the network,
    /// or an `ip:port` address).
    pub fn send_staged(self: &Arc<Self>, target: String) -> Result<(), String> {
        let paths = std::mem::take(&mut self.ui().staged);
        if paths.is_empty() {
            return Err("choose some files first".into());
        }
        let id = Uuid::new_v4().to_string();
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let peer_name = target
            .parse::<DeviceId>()
            .ok()
            .and_then(|device_id| self.ui().nearby.get(&device_id).map(|d| d.name.clone()))
            .unwrap_or_else(|| target.clone());
        self.add_transfer(Transfer {
            view: TransferView {
                id: id.clone(),
                direction: "send",
                peer_name,
                file_names: paths.iter().take(5).map(|p| file_name(p)).collect(),
                file_count: paths.len(),
                total_bytes: paths
                    .iter()
                    .filter_map(|p| std::fs::metadata(p).ok())
                    .map(|m| m.len())
                    .sum(),
                done_bytes: 0,
                state: TransferState::Waiting,
                detail: Some("Connecting…".into()),
                speed_bps: 0.0,
                eta_secs: None,
                verification_code: None,
                folder: None,
                started_ms: now_ms(),
            },
            rate: RateMeter::default(),
            cancel: Some(cancel_tx),
        });
        self.emit();
        let state = self.clone();
        tauri::async_runtime::spawn(async move {
            state.run_send(id, target, paths, cancel_rx).await;
        });
        Ok(())
    }

    async fn run_send(
        self: Arc<Self>,
        id: String,
        target: String,
        paths: Vec<PathBuf>,
        cancel: watch::Receiver<bool>,
    ) {
        let (addr, expect_device) = match self.resolve_target(&target).await {
            Ok(found) => found,
            Err(error) => {
                self.finish_transfer(&id, TransferState::Failed, Some(error), None);
                return;
            }
        };
        let options = SendOptions {
            streams: self.settings().streams,
            expect_device,
            cancel: Some(cancel),
            ..SendOptions::default()
        };
        let result = send_to(addr, &self.device, &self.trust, &paths, &options, |event| {
            self.on_send_event(&id, event)
        })
        .await;
        match result {
            Ok(SendOutcome::Completed(_)) => {
                self.finish_transfer(&id, TransferState::Done, None, None)
            }
            Ok(SendOutcome::Rejected { reason }) => {
                self.finish_transfer(&id, TransferState::Declined, Some(reason), None)
            }
            Err(TransferError::CancelledByUser) => {
                self.finish_transfer(&id, TransferState::Cancelled, None, None)
            }
            Err(error) => {
                self.finish_transfer(&id, TransferState::Failed, Some(error.to_string()), None)
            }
        }
    }

    async fn resolve_target(&self, target: &str) -> Result<(SocketAddr, Option<DeviceId>), String> {
        if let Ok(addr) = target.parse::<SocketAddr>() {
            return Ok((addr, None));
        }
        let device_id: DeviceId = target
            .parse()
            .map_err(|_| format!("{target:?} is neither a device nor an address"))?;
        let (name, addresses) = {
            let ui = self.ui();
            let device = ui
                .nearby
                .get(&device_id)
                .ok_or("that device is no longer on the network")?;
            (device.name.clone(), device.addresses.clone())
        };
        let addr = first_reachable(&addresses)
            .await
            .ok_or_else(|| format!("\"{name}\" didn't answer on any of its addresses"))?;
        Ok((addr, Some(device_id)))
    }

    fn on_send_event(&self, id: &str, event: SendEvent) {
        let throttled = matches!(event, SendEvent::Progress(_));
        self.update_transfer(id, |t| match event {
            SendEvent::Connected(peer) => {
                t.view.peer_name = peer.name().to_string();
                t.view.verification_code = Some(peer.verification_code.clone());
                t.view.detail = Some("Waiting for them to accept…".into());
            }
            SendEvent::Accepted { .. } => {
                t.view.state = TransferState::Active;
                t.view.detail = None;
            }
            SendEvent::Progress(progress) => apply_progress(t, progress),
            SendEvent::Reconnecting { attempt, .. } => {
                t.view.state = TransferState::Reconnecting;
                t.view.detail = Some(format!(
                    "Connection lost, reconnecting (attempt {attempt})…"
                ));
            }
        });
        if throttled {
            self.emit_throttled();
        } else {
            self.emit();
        }
    }

    fn finish_transfer(
        &self,
        id: &str,
        state: TransferState,
        detail: Option<String>,
        folder: Option<String>,
    ) {
        self.update_transfer(id, |t| {
            t.view.state = state;
            t.view.detail = detail;
            t.view.speed_bps = 0.0;
            t.view.eta_secs = None;
            t.cancel = None;
            if state == TransferState::Done {
                t.view.done_bytes = t.view.total_bytes;
            }
            if folder.is_some() {
                t.view.folder = folder;
            }
        });
        self.emit();
    }

    pub fn cancel_transfer(&self, id: &str) {
        let (send_cancel, receiving) = {
            let ui = self.ui();
            match ui.transfers.iter().find(|t| t.view.id == id) {
                Some(t) => (t.cancel.clone(), t.view.direction == "receive"),
                None => return,
            }
        };
        if let Some(cancel) = send_cancel {
            let _ = cancel.send(true);
        } else if receiving {
            if let (Ok(transfer_id), Some(receiver)) =
                (id.parse::<Uuid>(), lock(&self.receiver).as_ref())
            {
                receiver.handle.cancel(transfer_id);
            }
        }
    }

    pub fn clear_finished(&self) {
        self.ui().transfers.retain(|t| !t.view.state.is_finished());
        self.emit();
    }

    // ------------------------------------------------------------ incoming offers

    pub fn offer(&self, id: &str) -> Option<OfferView> {
        id.parse::<Uuid>()
            .ok()
            .and_then(|id| self.ui().offers.get(&id).map(|o| o.view.clone()))
    }

    /// The user answered the popup. `trust` also remembers the sender (with
    /// auto-accept) for next time.
    pub fn respond(&self, id: &str, accept: bool, trust: bool) -> Result<(), String> {
        let id: Uuid = id.parse().map_err(|_| "unknown offer")?;
        let (reply, name, key) = {
            let mut ui = self.ui();
            let offer = ui.offers.get_mut(&id).ok_or("this offer has expired")?;
            (offer.reply.take(), offer.peer_name.clone(), offer.peer_key)
        };
        if accept && trust {
            self.trust
                .upsert(TrustedDevice::new(name, key, true))
                .map_err(|e| e.to_string())?;
        }
        if let Some(reply) = reply {
            let _ = reply.send(if accept {
                Decision::Accept
            } else {
                Decision::Reject("declined".into())
            });
        }
        Ok(())
    }

    fn open_offer(&self, offer: &IncomingOffer, reply: oneshot::Sender<Decision>) {
        let id = offer.transfer_id;
        let view = OfferView {
            id: id.to_string(),
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
            expires_ms: now_ms() + OFFER_TIMEOUT.as_millis() as u64,
        };
        self.ui().offers.insert(
            id,
            PendingOffer {
                view,
                peer_name: offer.from.name().to_string(),
                peer_key: offer.from.public_key,
                reply: Some(reply),
            },
        );
        let _ = WebviewWindowBuilder::new(
            &self.app,
            offer_window_label(&id),
            WebviewUrl::App(format!("index.html#/offer/{id}").into()),
        )
        .title("Incoming files — Beam")
        .inner_size(420.0, 560.0)
        .resizable(false)
        .minimizable(false)
        .maximizable(false)
        .always_on_top(true)
        .center()
        .focused(true)
        .build();
        self.emit();
    }

    fn close_offer(&self, id: Uuid) {
        self.ui().offers.remove(&id);
        if let Some(window) = self.app.get_webview_window(&offer_window_label(&id)) {
            let _ = window.close();
        }
        self.emit();
    }

    // ------------------------------------------------------------ settings & trust

    pub fn set_visibility(&self, mode: VisibilityMode) {
        self.visibility.set(mode);
        crate::tray::show_visibility(&self.app, mode);
        self.emit();
    }

    pub fn set_auto_accept(&self, device_id: &str, auto_accept: bool) -> Result<(), String> {
        let id: DeviceId = device_id.parse().map_err(|e| format!("{e}"))?;
        let mut device = self
            .trust
            .get(&id)
            .map_err(|e| e.to_string())?
            .ok_or("not a trusted device")?;
        device.auto_accept = auto_accept;
        self.trust.upsert(device).map_err(|e| e.to_string())?;
        self.emit();
        Ok(())
    }

    pub fn remove_trusted(&self, device_id: &str) -> Result<(), String> {
        let id: DeviceId = device_id.parse().map_err(|e| format!("{e}"))?;
        self.trust.remove(&id).map_err(|e| e.to_string())?;
        self.emit();
        Ok(())
    }

    pub async fn update_settings(self: &Arc<Self>, new: Settings) -> Result<(), String> {
        let new = new.sanitized();
        let old = self.settings();
        new.save(&self.config_dir.join("settings.json"))
            .map_err(|e| format!("could not save settings: {e}"))?;
        *lock(&self.settings) = new.clone();

        if new.download_dir != old.download_dir {
            // Running transfers keep their folder; new ones use the new one.
            self.start_receiver().await?;
        }
        if new.send_to_shortcut != old.send_to_shortcut {
            crate::sendto::set_installed(new.send_to_shortcut)?;
        }
        if new.start_with_windows != old.start_with_windows {
            crate::set_autostart(&self.app, new.start_with_windows)?;
        }
        self.emit();
        Ok(())
    }

    pub fn download_dir(&self) -> PathBuf {
        self.settings().download_dir
    }
}

fn offer_window_label(id: &Uuid) -> String {
    format!("offer-{}", id.simple())
}

fn apply_progress(t: &mut Transfer, progress: Progress) {
    t.view.done_bytes = progress.bytes_done;
    t.view.total_bytes = progress.bytes_total;
    t.rate.update(progress.bytes_done);
    t.view.speed_bps = t.rate.bytes_per_sec;
    t.view.eta_secs = (t.rate.bytes_per_sec > 0.0).then(|| {
        progress.bytes_total.saturating_sub(progress.bytes_done) as f64 / t.rate.bytes_per_sec
    });
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------- receiving

/// Bridges beam-core's receiver to the UI.
struct DesktopHandler(Arc<AppState>);

impl ReceiveHandler for DesktopHandler {
    fn paired(&self, device: &beam_core::PeerInfo) {
        let _ = self.0.app.emit("paired", device.name().to_string());
        self.0.emit();
    }

    fn decide(&self, offer: IncomingOffer) -> BoxFuture<Decision> {
        let state = self.0.clone();
        Box::pin(async move {
            let (reply, answer) = oneshot::channel();
            let id = offer.transfer_id;
            state.open_offer(&offer, reply);
            let decision = match tokio::time::timeout(OFFER_TIMEOUT, answer).await {
                Ok(Ok(decision)) => decision,
                _ => Decision::Reject("no answer".into()),
            };
            state.close_offer(id);
            decision
        })
    }

    fn accepted(&self, offer: &IncomingOffer, resumed: bool) {
        let state = &self.0;
        let id = offer.transfer_id.to_string();
        let exists = state.ui().transfers.iter().any(|t| t.view.id == id);
        if exists && resumed {
            state.update_transfer(&id, |t| {
                t.view.state = TransferState::Active;
                t.view.detail = None;
            });
        } else {
            state.add_transfer(Transfer {
                view: TransferView {
                    id,
                    direction: "receive",
                    peer_name: offer.from.name().to_string(),
                    file_names: offer.files.iter().take(5).map(|f| f.name.clone()).collect(),
                    file_count: offer.files.len(),
                    total_bytes: offer.total_size,
                    done_bytes: 0,
                    state: TransferState::Active,
                    detail: None,
                    speed_bps: 0.0,
                    eta_secs: None,
                    verification_code: Some(offer.from.verification_code.clone()),
                    folder: Some(state.download_dir().display().to_string()),
                    started_ms: now_ms(),
                },
                rate: RateMeter::default(),
                cancel: None,
            });
        }
        state.emit();
    }

    fn progress(&self, transfer_id: Uuid, progress: Progress) {
        self.0
            .update_transfer(&transfer_id.to_string(), |t| apply_progress(t, progress));
        self.0.emit_throttled();
    }

    fn finished(&self, transfer_id: Uuid, result: &Result<ReceiveOutcome, TransferError>) {
        let id = transfer_id.to_string();
        let (state, detail) = match result {
            Ok(ReceiveOutcome::Completed { .. }) => (TransferState::Done, None),
            Ok(ReceiveOutcome::Rejected { reason, .. }) => {
                // Declined before anything was accepted: no row exists, so
                // there's nothing to update.
                let _ = reason;
                return self.0.emit();
            }
            Err(error) if error.is_retryable() || matches!(error, TransferError::Superseded) => (
                TransferState::Reconnecting,
                Some("Connection lost; waiting for the sender to reconnect…".to_string()),
            ),
            Err(TransferError::CancelledByUser) => (TransferState::Cancelled, None),
            Err(TransferError::Cancelled(reason)) => (
                TransferState::Cancelled,
                Some(format!("The sender cancelled ({reason})")),
            ),
            Err(error) => (TransferState::Failed, Some(error.to_string())),
        };
        if state == TransferState::Reconnecting {
            self.0.update_transfer(&id, |t| {
                t.view.state = state;
                t.view.detail = detail;
                t.view.speed_bps = 0.0;
                t.view.eta_secs = None;
            });
            self.0.emit();
        } else {
            self.0.finish_transfer(&id, state, detail, None);
        }
    }
}
