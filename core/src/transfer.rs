//! Types and helpers shared by the sender and the receiver.

use std::io;
use std::path::Path;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::connection::{Connection, ConnectionError};
use crate::frame::Frame;
use crate::identity::{Identity, KEY_LEN};
use crate::messages::{DeviceId, DeviceType, Hello, ProtocolVersion, PROTOCOL_VERSION};
use crate::secure::{secure_connection, HandshakeInfo, Role};
use crate::trust::{TrustStore, TrustedDevice};

/// Unit of hashing, resume and scheduling (SPEC.md section 7.3).
pub const CHUNK_SIZE: u64 = 4 * 1024 * 1024;
/// Keepalive: each side sends PING on the control connection this often...
pub const PING_INTERVAL: Duration = Duration::from_secs(5);
/// ...and gives up on a control connection that has been silent this long.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(15);
/// Receiver batches CHUNK_ACKs for at most this long.
pub const ACK_INTERVAL: Duration = Duration::from_millis(250);
/// Receiver saves its progress file at least this often.
pub const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(2);
/// Most parallel data connections per transfer.
pub const MAX_STREAMS: usize = 16;
/// A chunk that fails its hash this many times ends the transfer.
pub const MAX_CHUNK_RETRIES: u32 = 5;
/// Offers with more files than this are rejected.
pub const MAX_FILES_PER_OFFER: usize = 10_000;
/// Offers containing a file name longer than this (before sanitizing) are rejected.
pub const MAX_RAW_NAME_BYTES: usize = 1024;

/// This device: its key pair plus what it announces in HELLO.
#[derive(Debug, Clone)]
pub struct LocalDevice {
    pub identity: Identity,
    pub name: String,
    pub device_type: DeviceType,
}

impl LocalDevice {
    pub fn device_id(&self) -> DeviceId {
        self.identity.device_id()
    }

    pub fn hello(&self) -> Hello {
        Hello {
            version: PROTOCOL_VERSION,
            device_id: self.device_id(),
            name: self.name.clone(),
            device_type: self.device_type,
            capabilities: Vec::new(),
        }
    }
}

/// The other side of a session, after the handshake proved its key.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub hello: Hello,
    pub public_key: [u8; KEY_LEN],
    /// Same six digits on both screens unless someone is in the middle.
    pub verification_code: String,
    /// The trust-store entry for this key, if the user trusts it.
    pub trusted: Option<TrustedDevice>,
}

impl PeerInfo {
    pub fn device_id(&self) -> DeviceId {
        self.hello.device_id
    }

    pub fn name(&self) -> &str {
        &self.hello.name
    }

    pub fn is_trusted(&self) -> bool {
        self.trusted.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub files_done: usize,
    pub files_total: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TransferSummary {
    pub files: usize,
    /// Total size of the files in the transfer.
    pub bytes: u64,
    /// From ACCEPT to TRANSFER_COMPLETE (the user's decision time is excluded).
    pub duration: Duration,
    /// Connect + handshake + HELLO for the first connection.
    pub setup: Duration,
    /// Parallel data connections used.
    pub streams: usize,
    /// Times the connection dropped and was re-established.
    pub reconnects: u32,
    /// Chunks sent again because they failed verification.
    pub resent_chunks: u32,
    /// Bytes the receiver already had when the transfer (re)started.
    pub resumed_bytes: u64,
    /// Highest throughput over any one-second window, in MB/s (sender only).
    pub peak_mb_per_s: f64,
}

impl TransferSummary {
    /// Average throughput in MB/s, where 1 MB = 1,000,000 bytes.
    pub fn megabytes_per_second(&self) -> f64 {
        let seconds = self.duration.as_secs_f64();
        if seconds == 0.0 {
            return 0.0;
        }
        self.bytes as f64 / 1_000_000.0 / seconds
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error(transparent)]
    Connection(#[from] ConnectionError),
    #[error("{path}: {source}")]
    LocalFile { path: String, source: io::Error },
    #[error("peer speaks protocol {peer}, but this device speaks {ours}")]
    VersionMismatch {
        ours: ProtocolVersion,
        peer: ProtocolVersion,
    },
    #[error("protocol violation: {0}")]
    Protocol(String),
    #[error("invalid offer: {0}")]
    InvalidOffer(String),
    #[error("transfer cancelled by peer: {0}")]
    Cancelled(String),
    #[error("{file} changed while it was being sent")]
    FileChanged { file: String },
    #[error("hash mismatch in {file}{}", .chunk.map(|c| format!(" (chunk {c})")).unwrap_or_default())]
    HashMismatch { file: String, chunk: Option<u64> },
    #[error(
        "peer claims device id {claimed}, but its key belongs to {actual}; connection refused"
    )]
    IdentityMismatch { claimed: DeviceId, actual: DeviceId },
    #[error(
        "\"{name}\" ({device_id}) presented a different key than the one you trusted. \
         Someone may be impersonating it; connection refused"
    )]
    TrustedKeyMismatch { device_id: DeviceId, name: String },
    #[error(transparent)]
    TrustStore(#[from] crate::trust::TrustStoreError),
    #[error("the receiver's key changed after reconnecting; refusing to resume")]
    PeerChanged,
    #[error(
        "expected to reach device {expected}, but {actual} answered (another device          may be announcing the same name); nothing was sent"
    )]
    UnexpectedPeer {
        expected: DeviceId,
        actual: DeviceId,
    },
    #[error("chunk {chunk} of {file} failed verification {MAX_CHUNK_RETRIES} times")]
    TooManyRetries { file: String, chunk: u64 },
    #[error("could not reconnect within {seconds} s; last error: {last}")]
    ReconnectFailed {
        seconds: u64,
        last: Box<TransferError>,
    },
    #[error("a newer connection took over this transfer")]
    Superseded,
    #[error("transfer cancelled")]
    CancelledByUser,
    #[error("pairing refused: {0}")]
    PairingRefused(String),
    #[error("data connection rejected: {0}")]
    BadDataStream(String),
}

impl TransferError {
    /// Errors that mean "the network went away", after which the sender
    /// reconnects and resumes. Everything else (a decision, an attack, a
    /// protocol bug, a local file problem) ends the transfer.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            TransferError::Connection(
                ConnectionError::Io(_) | ConnectionError::Closed | ConnectionError::Timeout(_)
            )
        )
    }

    pub(crate) fn local_file(path: &Path) -> impl FnOnce(io::Error) -> TransferError + '_ {
        move |source| TransferError::LocalFile {
            path: path.display().to_string(),
            source,
        }
    }

    pub(crate) fn unexpected(expected: &str, got: &Frame) -> TransferError {
        TransferError::Protocol(format!("expected {expected}, got {}", got.name()))
    }
}

/// Number of chunks in a file of `size` bytes. An empty file has none.
pub fn chunk_count(size: u64) -> u64 {
    size.div_ceil(CHUNK_SIZE)
}

/// Offset just past the end of chunk `index`.
pub fn chunk_end(index: u64, size: u64) -> u64 {
    index.saturating_add(1).saturating_mul(CHUNK_SIZE).min(size)
}

/// Sender side of setting up a control connection:
///
/// 1. Noise XX handshake, which proves the peer holds its static key.
/// 2. Encrypted HELLO exchange (we speak first) and version check.
/// 3. Identity check (see [`verify_peer`]).
pub(crate) async fn open_control<S>(
    stream: S,
    me: &LocalDevice,
    trust: &TrustStore,
) -> Result<(Connection<S>, PeerInfo, HandshakeInfo), TransferError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut conn, handshake) = secure_connection(stream, &me.identity, Role::Initiator).await?;
    conn.send(&Frame::Hello(me.hello())).await?;
    let hello = match conn.read_frame().await? {
        Frame::Hello(hello) => hello,
        other => return Err(TransferError::unexpected("HELLO", &other)),
    };
    let peer = verify_peer(hello, &handshake, trust)?;
    Ok((conn, peer, handshake))
}

/// Receiver side, once the first frame on a new connection was a HELLO. We
/// reply with our HELLO before checking, so both ends can report a version
/// mismatch clearly.
pub(crate) async fn answer_control<S>(
    conn: &mut Connection<S>,
    hello: Hello,
    handshake: &HandshakeInfo,
    me: &LocalDevice,
    trust: &TrustStore,
) -> Result<PeerInfo, TransferError>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    conn.send(&Frame::Hello(me.hello())).await?;
    verify_peer(hello, handshake, trust)
}

/// Checks the peer's HELLO against what the handshake proved: the major
/// version must match, the device id must be derived from the proven key,
/// and if that id is trusted the key must be exactly the trusted one.
fn verify_peer(
    hello: Hello,
    handshake: &HandshakeInfo,
    trust: &TrustStore,
) -> Result<PeerInfo, TransferError> {
    if hello.version.major != PROTOCOL_VERSION.major {
        return Err(TransferError::VersionMismatch {
            ours: PROTOCOL_VERSION,
            peer: hello.version,
        });
    }
    let trusted = check_peer_identity(&hello, &handshake.remote_public_key, trust)?;
    Ok(PeerInfo {
        hello,
        public_key: handshake.remote_public_key,
        verification_code: handshake.verification_code(),
        trusted,
    })
}

fn check_peer_identity(
    hello: &Hello,
    proven_key: &[u8; KEY_LEN],
    trust: &TrustStore,
) -> Result<Option<TrustedDevice>, TransferError> {
    let actual = DeviceId::from_public_key(proven_key);
    if hello.device_id != actual {
        return Err(TransferError::IdentityMismatch {
            claimed: hello.device_id,
            actual,
        });
    }
    match trust.get(&hello.device_id)? {
        Some(trusted) if &trusted.public_key != proven_key => {
            Err(TransferError::TrustedKeyMismatch {
                device_id: trusted.device_id,
                name: trusted.name,
            })
        }
        trusted => Ok(trusted),
    }
}

/// Reads control frames in a task of their own and forwards them. Reading
/// isn't cancel-safe, so it can't sit directly in a `select!`. A control
/// connection that stays silent for [`IDLE_TIMEOUT`] counts as dead (both
/// sides PING every [`PING_INTERVAL`]). The first error ends the task.
pub(crate) fn spawn_control_reader<R>(
    tasks: &mut tokio::task::JoinSet<()>,
    mut reader: crate::connection::FrameReader<R>,
) -> tokio::sync::mpsc::Receiver<Result<Frame, TransferError>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    tasks.spawn(async move {
        loop {
            let result = match tokio::time::timeout(IDLE_TIMEOUT, reader.read_frame()).await {
                Ok(Ok(frame)) => Ok(frame),
                Ok(Err(e)) => Err(e.into()),
                Err(_) => Err(ConnectionError::Timeout("keepalive from the peer").into()),
            };
            let failed = result.is_err();
            if tx.send(result).await.is_err() || failed {
                break;
            }
        }
    });
    rx
}

/// Waits briefly for a CANCEL on the control connection. A peer that cancels
/// sends CANCEL and then closes everything, and its connections can look
/// broken here before the CANCEL is read: call this before treating a
/// broken connection as a network problem (which would wait for a resume).
pub(crate) async fn pending_cancel(
    frames: &mut tokio::sync::mpsc::Receiver<Result<Frame, TransferError>>,
) -> Option<String> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    while let Ok(Some(Ok(frame))) = tokio::time::timeout_at(deadline, frames.recv()).await {
        if let Frame::Cancel(cancel) = frame {
            return Some(cancel.reason);
        }
    }
    None
}

/// Positional write: `buf` at `offset`, independent of any file cursor, so
/// chunks can land in any order.
pub(crate) fn write_all_at(
    file: &std::fs::File,
    mut buf: &[u8],
    mut offset: u64,
) -> io::Result<()> {
    while !buf.is_empty() {
        #[cfg(unix)]
        let written = std::os::unix::fs::FileExt::write_at(file, buf, offset)?;
        #[cfg(windows)]
        let written = std::os::windows::fs::FileExt::seek_write(file, buf, offset)?;
        if written == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        buf = &buf[written..];
        offset += written as u64;
    }
    Ok(())
}

/// Positional read of exactly `buf.len()` bytes at `offset`.
pub(crate) fn read_exact_at(
    file: &std::fs::File,
    mut buf: &mut [u8],
    mut offset: u64,
) -> io::Result<()> {
    while !buf.is_empty() {
        #[cfg(unix)]
        let read = std::os::unix::fs::FileExt::read_at(file, buf, offset)?;
        #[cfg(windows)]
        let read = std::os::windows::fs::FileExt::seek_read(file, buf, offset)?;
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        buf = &mut buf[read..];
        offset += read as u64;
    }
    Ok(())
}

/// BLAKE3 of a whole file, using all CPU cores. Reads in fixed-size blocks
/// rather than memory-mapping, so memory use stays flat for huge files (a
/// mapped file counts toward the process's resident memory).
pub(crate) fn hash_file(path: &Path) -> io::Result<blake3::Hash> {
    let file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    hash_open_file(&file, len)
}

/// BLAKE3 of the first `len` bytes of an open file, with positional reads
/// (the file's cursor isn't touched, so other readers can share it).
pub(crate) fn hash_open_file(file: &std::fs::File, len: u64) -> io::Result<blake3::Hash> {
    const BLOCK: u64 = 4 * 1024 * 1024;
    let mut hasher = blake3::Hasher::new();
    let mut block = vec![0u8; BLOCK.min(len).max(1) as usize];
    let mut offset = 0;
    while offset < len {
        let n = (len - offset).min(BLOCK) as usize;
        read_exact_at(file, &mut block[..n], offset)?;
        hasher.update_rayon(&block[..n]);
        offset += n as u64;
    }
    Ok(hasher.finalize())
}

/// Tracks the best throughput over one-second windows.
#[derive(Debug, Default)]
pub(crate) struct PeakMeter {
    window_start: Option<std::time::Instant>,
    window_bytes: u64,
    peak: f64,
}

impl PeakMeter {
    pub(crate) fn add(&mut self, bytes: u64) {
        let now = std::time::Instant::now();
        let start = *self.window_start.get_or_insert(now);
        self.window_bytes += bytes;
        let elapsed = now - start;
        if elapsed >= Duration::from_secs(1) {
            let rate = self.window_bytes as f64 / 1_000_000.0 / elapsed.as_secs_f64();
            self.peak = self.peak.max(rate);
            self.window_start = Some(now);
            self.window_bytes = 0;
        }
    }

    /// Peak MB/s, or `fallback` (the average) for transfers shorter than a
    /// second.
    pub(crate) fn peak_or(&self, fallback: f64) -> f64 {
        if self.peak == 0.0 {
            fallback
        } else {
            self.peak.max(fallback)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_math() {
        assert_eq!(chunk_count(0), 0);
        assert_eq!(chunk_count(1), 1);
        assert_eq!(chunk_count(CHUNK_SIZE), 1);
        assert_eq!(chunk_count(CHUNK_SIZE + 1), 2);
        assert_eq!(chunk_end(0, 10), 10);
        assert_eq!(chunk_end(0, CHUNK_SIZE * 3), CHUNK_SIZE);
        assert_eq!(chunk_end(2, CHUNK_SIZE * 2 + 5), CHUNK_SIZE * 2 + 5);
        assert_eq!(chunk_end(u64::MAX, 7), 7);
    }

    fn hello_for(key: &[u8; KEY_LEN]) -> Hello {
        Hello {
            version: PROTOCOL_VERSION,
            device_id: DeviceId::from_public_key(key),
            name: "Pixel".into(),
            device_type: DeviceType::Phone,
            capabilities: vec![],
        }
    }

    #[test]
    fn peer_identity_checks() {
        let (real_key, other_key) = ([1u8; KEY_LEN], [2u8; KEY_LEN]);
        let trust = TrustStore::in_memory();

        // Unknown but consistent: allowed, not trusted.
        assert_eq!(
            check_peer_identity(&hello_for(&real_key), &real_key, &trust).unwrap(),
            None
        );

        // Claims an id that the proven key doesn't hash to.
        assert!(matches!(
            check_peer_identity(&hello_for(&real_key), &other_key, &trust),
            Err(TransferError::IdentityMismatch { .. })
        ));

        // Trusted id and matching key.
        let entry = TrustedDevice::new("Pixel".into(), real_key, true);
        trust.upsert(entry.clone()).unwrap();
        assert_eq!(
            check_peer_identity(&hello_for(&real_key), &real_key, &trust).unwrap(),
            Some(entry.clone())
        );

        // A trust entry whose stored key differs from the proven one (only
        // possible with a 64-bit id collision or an edited store) is refused.
        let forged = TrustedDevice {
            public_key: other_key,
            ..entry
        };
        trust.upsert(forged).unwrap();
        assert!(matches!(
            check_peer_identity(&hello_for(&real_key), &real_key, &trust),
            Err(TransferError::TrustedKeyMismatch { .. })
        ));
    }

    #[test]
    fn throughput_uses_decimal_megabytes() {
        let summary = TransferSummary {
            files: 1,
            bytes: 50_000_000,
            duration: Duration::from_secs(2),
            ..TransferSummary::default()
        };
        assert_eq!(summary.megabytes_per_second(), 25.0);
    }
}
