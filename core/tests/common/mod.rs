//! Helpers shared by the integration tests.

#![allow(dead_code)]

use std::fs;
use std::io::{BufWriter, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use beam_core::{
    BoxFuture, Decision, DeviceType, Identity, IncomingOffer, LocalDevice, Progress,
    ReceiveHandler, ReceiveOutcome, Receiver, ReceiverConfig, TransferError, TransferSummary,
    TrustStore, TrustedDevice, Visibility, VisibilityMode,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::{JoinHandle, JoinSet};
use uuid::Uuid;

pub fn device(name: &str) -> LocalDevice {
    LocalDevice {
        identity: Identity::generate(),
        name: name.to_string(),
        device_type: DeviceType::Laptop,
    }
}

pub fn trust_of(devices: &[(&LocalDevice, bool)]) -> TrustStore {
    let store = TrustStore::in_memory();
    for (device, auto_accept) in devices {
        store
            .upsert(TrustedDevice::new(
                device.name.clone(),
                *device.identity.public_key(),
                *auto_accept,
            ))
            .unwrap();
    }
    store
}

pub fn accept_all(_: &IncomingOffer) -> Decision {
    Decision::Accept
}

/// Writes `size` pseudo-random bytes (BLAKE3 in XOF mode, seeded by `seed`).
pub fn write_random_file(path: &Path, size: u64, seed: u64) {
    let mut output = blake3::Hasher::new()
        .update(&seed.to_le_bytes())
        .finalize_xof();
    let mut file = BufWriter::new(fs::File::create(path).unwrap());
    let mut buf = vec![0u8; 1 << 20];
    let mut remaining = size;
    while remaining > 0 {
        let n = remaining.min(buf.len() as u64) as usize;
        output.fill(&mut buf[..n]);
        file.write_all(&buf[..n]).unwrap();
        remaining -= n as u64;
    }
    file.flush().unwrap();
}

pub fn hash_file(path: &Path) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update_mmap_rayon(path).unwrap();
    hasher.finalize()
}

pub fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// A receiver outcome, flattened so it can cross a channel.
#[derive(Debug)]
pub enum Finished {
    Completed {
        from: String,
        auto_accepted: bool,
        resumed: bool,
        files: Vec<std::path::PathBuf>,
        summary: TransferSummary,
    },
    Rejected(String),
    Failed {
        error: String,
        retryable: bool,
    },
}

type DecideFn = dyn Fn(&IncomingOffer) -> Decision + Send + Sync;

struct TestHandler {
    decide: Arc<DecideFn>,
    finished: mpsc::UnboundedSender<Finished>,
    refused: mpsc::UnboundedSender<String>,
    progress: Arc<AtomicU64>,
}

impl ReceiveHandler for TestHandler {
    fn decide(&self, offer: IncomingOffer) -> BoxFuture<Decision> {
        let decision = (self.decide)(&offer);
        Box::pin(async move { decision })
    }

    fn progress(&self, _: Uuid, progress: Progress) {
        self.progress
            .fetch_max(progress.bytes_done, Ordering::Relaxed);
    }

    fn finished(&self, _: Uuid, result: &Result<ReceiveOutcome, TransferError>) {
        let flat = match result {
            Ok(ReceiveOutcome::Completed {
                from,
                auto_accepted,
                resumed,
                files,
                summary,
            }) => Finished::Completed {
                from: from.name().to_string(),
                auto_accepted: *auto_accepted,
                resumed: *resumed,
                files: files.iter().map(|f| f.path.clone()).collect(),
                summary: *summary,
            },
            Ok(ReceiveOutcome::Rejected { reason, .. }) => Finished::Rejected(reason.clone()),
            Err(e) => Finished::Failed {
                error: e.to_string(),
                retryable: e.is_retryable() || matches!(e, TransferError::Superseded),
            },
        };
        let _ = self.finished.send(flat);
    }

    fn connection_refused(&self, error: &TransferError) {
        let _ = self.refused.send(error.to_string());
    }
}

/// A receiver running in the background. Dropping it stops accepting new
/// connections.
pub struct TestReceiver {
    pub addr: SocketAddr,
    pub progress: Arc<AtomicU64>,
    finished: mpsc::UnboundedReceiver<Finished>,
    refused: mpsc::UnboundedReceiver<String>,
    task: JoinHandle<std::io::Result<()>>,
}

impl TestReceiver {
    pub async fn start(
        me: LocalDevice,
        trust: TrustStore,
        download_dir: &Path,
        decide: impl Fn(&IncomingOffer) -> Decision + Send + Sync + 'static,
    ) -> TestReceiver {
        let visibility = Visibility::new(VisibilityMode::Everyone);
        Self::start_with(me, trust, download_dir, visibility, decide).await
    }

    pub async fn start_with(
        me: LocalDevice,
        trust: TrustStore,
        download_dir: &Path,
        visibility: Visibility,
        decide: impl Fn(&IncomingOffer) -> Decision + Send + Sync + 'static,
    ) -> TestReceiver {
        let receiver = Receiver::bind(
            "127.0.0.1:0",
            ReceiverConfig {
                device: me,
                download_dir: download_dir.to_path_buf(),
                trust,
                visibility,
            },
        )
        .await
        .unwrap();
        let addr = receiver.local_addr().unwrap();
        let (finished_tx, finished) = mpsc::unbounded_channel();
        let (refused_tx, refused) = mpsc::unbounded_channel();
        let progress = Arc::new(AtomicU64::new(0));
        let task = tokio::spawn(receiver.run(TestHandler {
            decide: Arc::new(decide),
            finished: finished_tx,
            refused: refused_tx,
            progress: progress.clone(),
        }));
        TestReceiver {
            addr,
            progress,
            finished,
            refused,
            task,
        }
    }

    /// The final outcome of the next transfer, skipping sessions that were
    /// merely interrupted (and later resumed).
    pub async fn next_outcome(&mut self) -> Finished {
        loop {
            let next = tokio::time::timeout(Duration::from_secs(120), self.finished.recv())
                .await
                .expect("receiver finished in time")
                .expect("receiver is running");
            if !matches!(
                next,
                Finished::Failed {
                    retryable: true,
                    ..
                }
            ) {
                return next;
            }
        }
    }

    pub async fn next_refusal(&mut self) -> String {
        tokio::time::timeout(Duration::from_secs(10), self.refused.recv())
            .await
            .expect("refusal in time")
            .expect("receiver is running")
    }
}

impl Drop for TestReceiver {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A TCP proxy between sender and receiver that can record traffic, cut
/// every connection on demand, or cut them at random ("chaos").
pub struct Proxy {
    pub addr: SocketAddr,
    state: Arc<ProxyState>,
    accept_task: JoinHandle<()>,
}

struct ProxyState {
    backend: Mutex<SocketAddr>,
    recording: Option<Mutex<Vec<u8>>>,
    pipes: Mutex<JoinSet<()>>,
    forwarded: AtomicU64,
    chaos: Mutex<Option<Chaos>>,
    kills: AtomicU64,
}

struct Chaos {
    next_kill_at: u64,
    kills_left: u32,
    rng: u64,
}

impl Chaos {
    /// Next cut after 4–28 MB more traffic (xorshift, so runs repeat). That
    /// averages 16 MB, so 50 cuts always land within a 1 GB transfer.
    fn schedule(&mut self, now: u64) {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.next_kill_at = now + 4_000_000 + self.rng % 24_000_000;
    }
}

impl Proxy {
    pub async fn start(backend: SocketAddr, record: bool) -> Proxy {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(ProxyState {
            backend: Mutex::new(backend),
            recording: record.then(|| Mutex::new(Vec::new())),
            pipes: Mutex::new(JoinSet::new()),
            forwarded: AtomicU64::new(0),
            chaos: Mutex::new(None),
            kills: AtomicU64::new(0),
        });
        let accept_state = state.clone();
        let accept_task = tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else {
                    continue;
                };
                let backend = *accept_state.backend.lock().unwrap();
                let Ok(server) = TcpStream::connect(backend).await else {
                    continue;
                };
                let _ = client.set_nodelay(true);
                let _ = server.set_nodelay(true);
                let (client_read, client_write) = client.into_split();
                let (server_read, server_write) = server.into_split();
                let mut pipes = accept_state.pipes.lock().unwrap();
                while pipes.try_join_next().is_some() {}
                pipes.spawn(pipe(client_read, server_write, accept_state.clone()));
                pipes.spawn(pipe(server_read, client_write, accept_state.clone()));
            }
        });
        Proxy {
            addr,
            state,
            accept_task,
        }
    }

    pub fn enable_chaos(&self, kills: u32, seed: u64) {
        let mut chaos = Chaos {
            next_kill_at: 0,
            kills_left: kills,
            rng: seed | 1,
        };
        chaos.schedule(self.state.forwarded.load(Ordering::Relaxed));
        *self.state.chaos.lock().unwrap() = Some(chaos);
    }

    pub fn kills_done(&self) -> u64 {
        self.state.kills.load(Ordering::Relaxed)
    }

    /// Cuts every open connection at once.
    pub fn kill_all(&self) {
        self.state.kill_all();
    }

    pub fn set_backend(&self, backend: SocketAddr) {
        *self.state.backend.lock().unwrap() = backend;
    }

    pub fn recorded(&self) -> Vec<u8> {
        self.state
            .recording
            .as_ref()
            .expect("proxy started with recording")
            .lock()
            .unwrap()
            .clone()
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.accept_task.abort();
        self.state.kill_all();
    }
}

impl ProxyState {
    fn kill_all(&self) {
        self.pipes.lock().unwrap().abort_all();
    }
}

async fn pipe(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    state: Arc<ProxyState>,
) {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if let Some(recording) = &state.recording {
            recording.lock().unwrap().extend_from_slice(&buf[..n]);
        }
        if to.write_all(&buf[..n]).await.is_err() {
            break;
        }
        let total = state.forwarded.fetch_add(n as u64, Ordering::Relaxed) + n as u64;
        let kill = {
            let mut chaos = state.chaos.lock().unwrap();
            match chaos.as_mut() {
                Some(c) if c.kills_left > 0 && total >= c.next_kill_at => {
                    c.kills_left -= 1;
                    c.schedule(total);
                    true
                }
                _ => false,
            }
        };
        if kill {
            state.kills.fetch_add(1, Ordering::Relaxed);
            // Aborting from a separate task: this pipe is one of the victims.
            let state = state.clone();
            tokio::spawn(async move { state.kill_all() });
            return;
        }
    }
    let _ = to.shutdown().await;
}
