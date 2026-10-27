//! Types and helpers shared by the sender and the receiver.

use std::io;
use std::path::Path;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::connection::{Connection, ConnectionError, RecordCipher};
use crate::frame::Frame;
use crate::identity::{Identity, KEY_LEN};
use crate::messages::{DeviceId, DeviceType, Hello, ProtocolVersion, PROTOCOL_VERSION};
use crate::secure::{secure_connection, NoiseCipher, Role};
use crate::trust::{TrustStore, TrustedDevice};

/// Unit of hashing, resume and scheduling (SPEC.md section 7.3).
pub const CHUNK_SIZE: u64 = 4 * 1024 * 1024;
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransferSummary {
    pub files: usize,
    pub bytes: u64,
    pub duration: Duration,
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
}

impl TransferError {
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

/// Sets up a session on a fresh TCP stream:
///
/// 1. Noise XX handshake, which proves the peer holds its static key.
/// 2. Encrypted HELLO exchange and version check.
/// 3. Identity check: the device id in HELLO must be derived from the key the
///    handshake proved, and if that id is trusted, the key must be exactly the
///    trusted one. Otherwise the connection is refused.
pub(crate) async fn establish_session<S>(
    stream: S,
    me: &LocalDevice,
    trust: &TrustStore,
    role: Role,
) -> Result<(Connection<S, NoiseCipher>, PeerInfo), TransferError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (mut conn, handshake) = secure_connection(stream, &me.identity, role).await?;
    let hello = exchange_hello(&mut conn, me, role == Role::Initiator).await?;
    let trusted = check_peer_identity(&hello, &handshake.remote_public_key, trust)?;
    Ok((
        conn,
        PeerInfo {
            hello,
            public_key: handshake.remote_public_key,
            verification_code: handshake.verification_code(),
            trusted,
        },
    ))
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

/// Exchanges HELLO frames and checks the peer's major version. The side that
/// opened the connection speaks first; the other replies before checking, so
/// both ends can report a version mismatch clearly.
async fn exchange_hello<S, C>(
    conn: &mut Connection<S, C>,
    me: &LocalDevice,
    speak_first: bool,
) -> Result<Hello, TransferError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    C: RecordCipher,
{
    if speak_first {
        conn.send(&Frame::Hello(me.hello())).await?;
    }
    let peer = match conn.read_frame().await? {
        Frame::Hello(hello) => hello,
        other => return Err(TransferError::unexpected("HELLO", &other)),
    };
    if !speak_first {
        conn.send(&Frame::Hello(me.hello())).await?;
    }
    if peer.version.major != PROTOCOL_VERSION.major {
        return Err(TransferError::VersionMismatch {
            ours: PROTOCOL_VERSION,
            peer: peer.version,
        });
    }
    Ok(peer)
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
        };
        assert_eq!(summary.megabytes_per_second(), 25.0);
    }
}
