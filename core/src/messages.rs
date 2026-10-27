//! Message bodies from SPEC.md section 7.2.
//!
//! Every control message is a CBOR map with text keys (serde's default struct
//! encoding). Unknown keys are ignored when decoding, so a newer minor version
//! can add fields without breaking older peers.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The protocol version this build speaks. Peers with a different `major`
/// refuse to talk; a different `minor` is fine.
pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion { major: 1, minor: 0 };

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// Short device identifier: the first 8 bytes of SHA-256(static public key)
/// once identities exist (phase 2). Shown as 4 groups of 4 hex characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(#[serde(with = "serde_bytes")] pub [u8; 8]);

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, pair) in self.0.chunks(2).enumerate() {
            if i > 0 {
                f.write_str("-")?;
            }
            write!(f, "{:02x}{:02x}", pair[0], pair[1])?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceType {
    Phone,
    Laptop,
}

/// A BLAKE3 digest, encoded as a 32-byte CBOR byte string.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hash32(#[serde(with = "serde_bytes")] pub [u8; 32]);

impl From<blake3::Hash> for Hash32 {
    fn from(hash: blake3::Hash) -> Self {
        Hash32(*hash.as_bytes())
    }
}

impl fmt::Debug for Hash32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0[..8] {
            write!(f, "{byte:02x}")?;
        }
        f.write_str("…")
    }
}

/// 0x01, both directions: the first frame each side sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub version: ProtocolVersion,
    pub device_id: DeviceId,
    pub name: String,
    #[serde(rename = "type")]
    pub device_type: DeviceType,
    pub capabilities: Vec<String>,
}

/// One file inside an [`Offer`]. `name` is whatever the sender claims and must
/// be sanitized before it touches the file system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub file_id: u32,
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub modified_unix: i64,
}

/// 0x02, sender → receiver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    pub transfer_id: Uuid,
    pub files: Vec<FileEntry>,
    pub total_size: u64,
}

/// Chunks of one file the receiver already has verified (for resume).
/// Bit `i` is chunk `i`: byte `i / 8`, bit `i % 8`, least significant bit first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHave {
    pub file_id: u32,
    #[serde(with = "serde_bytes")]
    pub chunk_bitmap: Vec<u8>,
}

/// 0x03, receiver → sender.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accept {
    pub transfer_id: Uuid,
    pub have: Vec<FileHave>,
}

/// 0x04, receiver → sender.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reject {
    pub transfer_id: Uuid,
    pub reason: String,
}

/// 0x05, sender → receiver, first frame on a data connection (phase 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamOpen {
    pub transfer_id: Uuid,
    #[serde(with = "serde_bytes")]
    pub session_token: [u8; 32],
    pub stream_index: u16,
}

/// 0x06, sender → receiver. Encoded with a fixed binary header, not CBOR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Data {
    pub file_id: u32,
    pub offset: u64,
    pub bytes: Vec<u8>,
}

/// 0x07, sender → receiver, after the last DATA frame of a chunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkHash {
    pub file_id: u32,
    pub chunk_index: u64,
    pub blake3: Hash32,
}

/// 0x08, receiver → sender.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkAck {
    pub file_id: u32,
    pub chunk_indices: Vec<u64>,
}

/// 0x09, sender → receiver, after the last CHUNK_HASH of a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDone {
    pub file_id: u32,
    pub blake3_full: Hash32,
}

/// Receiver-side numbers reported in [`TransferComplete`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferStats {
    pub bytes: u64,
    pub files: u32,
    pub duration_ms: u64,
}

/// 0x0A, receiver → sender, once every file is verified and saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferComplete {
    pub transfer_id: Uuid,
    pub stats: TransferStats,
}

/// 0x0B, both directions: abort the transfer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cancel {
    pub transfer_id: Uuid,
    pub reason: String,
}

/// 0x0C / 0x0D, both directions: keepalive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ping {
    pub nonce: u64,
}

/// 0x10, both directions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clipboard {
    pub mime: String,
    pub text: String,
}
