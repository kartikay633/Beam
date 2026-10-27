//! Types and helpers shared by the sender and the receiver.

use std::io;
use std::path::Path;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::connection::{Connection, ConnectionError, RecordCipher};
use crate::frame::Frame;
use crate::messages::{DeviceId, DeviceType, Hello, ProtocolVersion, PROTOCOL_VERSION};

/// Unit of hashing, resume and scheduling (SPEC.md section 7.3).
pub const CHUNK_SIZE: u64 = 4 * 1024 * 1024;
/// Offers with more files than this are rejected.
pub const MAX_FILES_PER_OFFER: usize = 10_000;
/// Offers containing a file name longer than this (before sanitizing) are rejected.
pub const MAX_RAW_NAME_BYTES: usize = 1024;

/// Who this device is, as announced in HELLO.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub device_type: DeviceType,
}

impl DeviceInfo {
    pub fn hello(&self) -> Hello {
        Hello {
            version: PROTOCOL_VERSION,
            device_id: self.id,
            name: self.name.clone(),
            device_type: self.device_type,
            capabilities: Vec::new(),
        }
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

/// Exchanges HELLO frames and checks the peer's major version. The side that
/// opened the connection speaks first; the other replies before checking, so
/// both ends can report a version mismatch clearly.
pub(crate) async fn exchange_hello<S, C>(
    conn: &mut Connection<S, C>,
    me: &DeviceInfo,
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
