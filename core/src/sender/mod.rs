//! The sending side of a transfer.
//!
//! One *session* is a control connection plus N data connections (see
//! [`session`]). If the network drops after the receiver accepted, the sender
//! reconnects and resumes for up to [`SendOptions::reconnect_window`]; the
//! receiver's ACCEPT then lists the chunks it already has, and only the
//! missing ones are sent.

mod queue;
mod session;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use tokio::net::ToSocketAddrs;
use tokio::sync::{watch, Notify};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::connection::ConnectionError;
use crate::messages::{DeviceId, FileEntry, Hash32};
use crate::transfer::{
    chunk_count, hash_open_file, read_exact_at, LocalDevice, PeakMeter, PeerInfo, Progress,
    TransferError, TransferSummary,
};
use crate::trust::TrustStore;

pub(crate) use queue::{ChunkQueue, ChunkRef};

#[derive(Debug, Clone)]
pub struct SendOptions {
    /// Parallel data connections.
    pub streams: usize,
    /// How long to keep trying to reconnect after the connection drops.
    pub reconnect_window: Duration,
    /// The device the sender means to reach (for example one found by name
    /// over mDNS, which anyone can spoof). The handshake must prove this id,
    /// or nothing is sent.
    pub expect_device: Option<DeviceId>,
    /// Set to `true` to cancel the transfer: the receiver is told (CANCEL)
    /// and `send_to` returns [`TransferError::CancelledByUser`].
    pub cancel: Option<watch::Receiver<bool>>,
    /// Test hook: flip one byte in the first transmission of this
    /// `(file_id, chunk_index)` so the receiver's check fails once.
    #[doc(hidden)]
    pub corrupt_once: Option<(u32, u64)>,
}

impl Default for SendOptions {
    fn default() -> Self {
        SendOptions {
            streams: 4,
            reconnect_window: Duration::from_secs(60),
            expect_device: None,
            cancel: None,
            corrupt_once: None,
        }
    }
}

/// What the sender reports while it works.
#[derive(Debug, Clone)]
pub enum SendEvent {
    /// The encrypted session is up and the receiver's identity is verified.
    /// Show the verification code now, while the receiver decides.
    Connected(PeerInfo),
    /// The receiver accepted (or, after a reconnect, resumed) the transfer.
    Accepted {
        resumed_bytes: u64,
    },
    Progress(Progress),
    /// The connection dropped; trying again.
    Reconnecting {
        attempt: u32,
        error: String,
    },
}

#[derive(Debug)]
pub enum SendOutcome {
    Completed(TransferSummary),
    Rejected { reason: String },
}

/// Something to send: a file path, or a file that is already open (for
/// example an Android content URI opened as a file descriptor).
#[derive(Debug, Clone)]
pub struct SendSource {
    kind: SourceKind,
}

#[derive(Debug, Clone)]
enum SourceKind {
    Path(PathBuf),
    Open {
        file: Arc<std::fs::File>,
        name: String,
        modified_unix: i64,
    },
}

impl SendSource {
    pub fn path(path: impl Into<PathBuf>) -> SendSource {
        SendSource {
            kind: SourceKind::Path(path.into()),
        }
    }

    /// An open, seekable file. It is read with positional reads, so it must
    /// be a regular file (not a pipe). `name` is what the receiver sees.
    pub fn open_file(
        file: std::fs::File,
        name: impl Into<String>,
        modified_unix: i64,
    ) -> SendSource {
        SendSource {
            kind: SourceKind::Open {
                file: Arc::new(file),
                name: name.into(),
                modified_unix,
            },
        }
    }

    /// How errors refer to it.
    pub(crate) fn label(&self) -> PathBuf {
        match &self.kind {
            SourceKind::Path(path) => path.clone(),
            SourceKind::Open { name, .. } => PathBuf::from(name),
        }
    }

    /// Reads exactly `buf.len()` bytes at `offset` (blocking).
    pub(crate) fn read_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
        match &self.kind {
            SourceKind::Path(path) => read_exact_at(&std::fs::File::open(path)?, buf, offset),
            SourceKind::Open { file, .. } => read_exact_at(file, buf, offset),
        }
    }

    /// BLAKE3 of the first `len` bytes (blocking).
    pub(crate) fn hash(&self, len: u64) -> std::io::Result<blake3::Hash> {
        match &self.kind {
            SourceKind::Path(path) => hash_open_file(&std::fs::File::open(path)?, len),
            SourceKind::Open { file, .. } => hash_open_file(file, len),
        }
    }
}

/// One file of the transfer, as offered.
#[derive(Debug, Clone)]
pub(crate) struct OutgoingFile {
    pub entry: FileEntry,
    pub source: SendSource,
    pub chunks: u64,
}

/// Everything that stays the same across reconnects.
pub(crate) struct TransferPlan {
    pub transfer_id: Uuid,
    pub files: Vec<OutgoingFile>,
    pub total_size: u64,
}

/// Whole-file BLAKE3 hashes, computed in the background (each file is read
/// once more, using all cores) so FILE_DONE can be sent without delaying the
/// start of the transfer.
pub(crate) struct FullHashes {
    results: Mutex<Vec<Option<Result<Hash32, String>>>>,
    pub ready: Notify,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl FullHashes {
    /// Starts hashing every multi-chunk file in the background. Empty files
    /// are known at once; one-chunk files get their hash from the stream that
    /// sends them (see [`FullHashes::set`]).
    fn spawn(plan: &TransferPlan) -> Arc<FullHashes> {
        let hashes = Arc::new(FullHashes {
            results: Mutex::new(vec![None; plan.files.len()]),
            ready: Notify::new(),
            tasks: Mutex::new(Vec::new()),
        });
        for (index, file) in plan.files.iter().enumerate() {
            if file.chunks == 0 {
                hashes.set(index, blake3::hash(b"").into());
            }
        }
        hashes.compute(
            plan,
            (0..plan.files.len())
                .filter(|&i| plan.files[i].chunks > 1)
                .collect(),
        );
        hashes
    }

    /// Hashes the given files from disk, one after another, in the background.
    pub(crate) fn compute(self: &Arc<Self>, plan: &TransferPlan, files: Vec<usize>) {
        if files.is_empty() {
            return;
        }
        let jobs: Vec<(usize, SendSource, u64)> = files
            .into_iter()
            .map(|i| (i, plan.files[i].source.clone(), plan.files[i].entry.size))
            .collect();
        let worker = self.clone();
        let task = tokio::spawn(async move {
            for (index, source, len) in jobs {
                let result = tokio::task::spawn_blocking(move || {
                    source
                        .hash(len)
                        .map(Hash32::from)
                        .map_err(|e| format!("{}: {e}", source.label().display()))
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
                worker.results.lock().unwrap_or_else(|e| e.into_inner())[index] = Some(result);
                worker.ready.notify_one();
            }
        });
        self.tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(task);
    }

    /// Records a hash computed elsewhere (by the stream that sent the file).
    pub(crate) fn set(&self, index: usize, hash: Hash32) {
        self.results.lock().unwrap_or_else(|e| e.into_inner())[index] = Some(Ok(hash));
        self.ready.notify_one();
    }

    /// The hash of file `index`, if it's ready.
    pub(crate) fn get(&self, index: usize) -> Option<Result<Hash32, String>> {
        self.results.lock().unwrap_or_else(|e| e.into_inner())[index].clone()
    }
}

impl Drop for FullHashes {
    fn drop(&mut self) {
        for task in self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
        {
            task.abort();
        }
    }
}

/// State carried from one session to the next.
#[derive(Default)]
pub(crate) struct SessionHistory {
    /// The receiver's key from the first session: a reconnect must reach the
    /// same device.
    pub pinned_key: Option<[u8; 32]>,
    /// The receiver accepted at least once, so dropped connections are
    /// worth retrying.
    pub accepted_once: bool,
    /// The most recent session got as far as ACCEPT.
    pub session_accepted: bool,
    pub setup: Option<Duration>,
    pub first_accept: Option<Instant>,
    pub resent_chunks: u32,
    pub resumed_bytes: u64,
    pub streams: usize,
    pub peak: PeakMeter,
    /// Pending test-hook corruption, consumed by the first matching send.
    pub corrupt_once: Arc<Mutex<Option<(u32, u64)>>>,
}

/// Checks the files, connects to `addr`, and sends them, reconnecting and
/// resuming if the connection drops after the receiver accepted.
pub async fn send_to(
    addr: impl ToSocketAddrs,
    me: &LocalDevice,
    trust: &TrustStore,
    paths: &[PathBuf],
    options: &SendOptions,
    on_event: impl FnMut(SendEvent),
) -> Result<SendOutcome, TransferError> {
    let sources = paths.iter().cloned().map(SendSource::path).collect();
    send_sources(addr, me, trust, sources, options, on_event).await
}

/// Like [`send_to`], for paths and already-open files alike.
pub async fn send_sources(
    addr: impl ToSocketAddrs,
    me: &LocalDevice,
    trust: &TrustStore,
    sources: Vec<SendSource>,
    options: &SendOptions,
    mut on_event: impl FnMut(SendEvent),
) -> Result<SendOutcome, TransferError> {
    // Fail on a bad file before touching the network.
    let files = describe_files(sources).await?;
    let addr = resolve(addr).await?;
    let plan = Arc::new(TransferPlan {
        transfer_id: Uuid::new_v4(),
        total_size: files.iter().map(|f| f.entry.size).sum(),
        files,
    });
    let hashes = FullHashes::spawn(&plan);
    let mut history = SessionHistory {
        corrupt_once: Arc::new(Mutex::new(options.corrupt_once)),
        ..SessionHistory::default()
    };

    let mut failing_since: Option<Instant> = None;
    let mut attempt = 0u32;
    let mut reconnects = 0u32;
    loop {
        history.session_accepted = false;
        let result = session::run(
            addr,
            me,
            trust,
            &plan,
            &hashes,
            options,
            &mut history,
            &mut on_event,
        )
        .await;
        match result {
            Ok(session::SessionEnd::Completed) => {
                let duration = history.first_accept.map_or(Duration::ZERO, |t| t.elapsed());
                let mut summary = TransferSummary {
                    files: plan.files.len(),
                    bytes: plan.total_size,
                    duration,
                    setup: history.setup.unwrap_or_default(),
                    streams: history.streams,
                    reconnects,
                    resent_chunks: history.resent_chunks,
                    resumed_bytes: history.resumed_bytes,
                    peak_mb_per_s: 0.0,
                };
                summary.peak_mb_per_s = history.peak.peak_or(summary.megabytes_per_second());
                return Ok(SendOutcome::Completed(summary));
            }
            Ok(session::SessionEnd::Rejected(reason)) => {
                return Ok(SendOutcome::Rejected { reason })
            }
            Err(err) if history.accepted_once && err.is_retryable() => {
                if history.session_accepted {
                    // An established transfer just dropped: start the clock.
                    reconnects += 1;
                    attempt = 0;
                    failing_since = Some(Instant::now());
                }
                let since = *failing_since.get_or_insert_with(Instant::now);
                if since.elapsed() > options.reconnect_window {
                    return Err(TransferError::ReconnectFailed {
                        seconds: options.reconnect_window.as_secs(),
                        last: Box::new(err),
                    });
                }
                attempt += 1;
                on_event(SendEvent::Reconnecting {
                    attempt,
                    error: err.to_string(),
                });
                let mut cancel = options.cancel.clone();
                tokio::select! {
                    _ = tokio::time::sleep(backoff(attempt)) => {}
                    _ = cancelled(&mut cancel) => return Err(TransferError::CancelledByUser),
                }
            }
            Err(err) => return Err(err),
        }
    }
}

/// Resolves once `cancel` is set to `true`; never, if there's no cancel handle.
pub(crate) async fn cancelled(cancel: &mut Option<watch::Receiver<bool>>) {
    match cancel {
        Some(rx) => {
            let _ = rx.wait_for(|cancelled| *cancelled).await;
        }
        None => std::future::pending().await,
    }
}

/// 100 ms, 200 ms, 400 ms, 800 ms, then 1.6 s between attempts.
fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(100 << attempt.clamp(1, 5).saturating_sub(1))
}

async fn resolve(addr: impl ToSocketAddrs) -> Result<SocketAddr, TransferError> {
    tokio::net::lookup_host(addr)
        .await
        .map_err(ConnectionError::from)?
        .next()
        .ok_or_else(|| {
            ConnectionError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "address did not resolve",
            ))
            .into()
        })
}

async fn describe_files(sources: Vec<SendSource>) -> Result<Vec<OutgoingFile>, TransferError> {
    let mut files = Vec::with_capacity(sources.len());
    for (index, source) in sources.into_iter().enumerate() {
        let label = source.label();
        let metadata = match &source.kind {
            SourceKind::Path(path) => tokio::fs::metadata(path).await,
            SourceKind::Open { file, .. } => file.metadata(),
        }
        .map_err(TransferError::local_file(&label))?;
        if !metadata.is_file() {
            return Err(TransferError::local_file(&label)(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a regular file",
            )));
        }
        let (name, modified_unix) = match &source.kind {
            SourceKind::Path(path) => (
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "file".to_string()),
                metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs() as i64),
            ),
            SourceKind::Open {
                name,
                modified_unix,
                ..
            } => (name.clone(), *modified_unix),
        };
        let file_id = u32::try_from(index)
            .map_err(|_| TransferError::InvalidOffer("too many files".to_string()))?;
        files.push(OutgoingFile {
            entry: FileEntry {
                file_id,
                mime: guess_mime(&name).to_string(),
                name,
                size: metadata.len(),
                modified_unix,
            },
            source,
            chunks: chunk_count(metadata.len()),
        });
    }
    Ok(files)
}

/// Best-effort MIME type from the extension. Receivers only use it for display.
fn guess_mime(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase());
    match ext.as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("heic") => "image/heic",
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("mp3") => "audio/mpeg",
        Some("pdf") => "application/pdf",
        Some("zip") => "application/zip",
        Some("txt") => "text/plain",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_guessing() {
        assert_eq!(guess_mime("IMG_001.JPG"), "image/jpeg");
        assert_eq!(guess_mime("notes.txt"), "text/plain");
        assert_eq!(guess_mime("archive.tar.gz"), "application/octet-stream");
        assert_eq!(guess_mime("README"), "application/octet-stream");
    }

    #[test]
    fn backoff_grows_then_caps() {
        assert_eq!(backoff(1), Duration::from_millis(100));
        assert_eq!(backoff(2), Duration::from_millis(200));
        assert_eq!(backoff(3), Duration::from_millis(400));
        assert_eq!(backoff(5), Duration::from_millis(1600));
        assert_eq!(backoff(100), Duration::from_millis(1600));
    }
}
