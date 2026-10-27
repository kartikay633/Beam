//! Noise XX session encryption (SPEC.md section 6.2), built on `snow`.
//!
//! Handshake messages use the same record format as everything else
//! (`len:u16 | message`). After the three XX messages both sides know each
//! other's static public key, and every later record is one Noise transport
//! message, produced by [`NoiseCipher`].

use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::connection::{CipherError, Connection, ConnectionError, RecordCipher, MAX_RECORD_LEN};
use crate::identity::{Identity, KEY_LEN};

pub const NOISE_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
/// Mixed into the handshake so a Beam handshake can never be confused with
/// another protocol that uses the same Noise pattern.
const PROLOGUE: &[u8] = b"beam/1";
/// A peer that stalls mid-handshake is dropped after this long.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const AEAD_TAG_LEN: usize = 16;
const VERIFICATION_CODE_CONTEXT: &str = "beam 2026-10-27 verification code v1";

pub(crate) fn noise_params() -> snow::params::NoiseParams {
    NOISE_PATTERN.parse().expect("valid Noise pattern")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The side that opened the TCP connection.
    Initiator,
    Responder,
}

/// What both sides learn from a completed handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandshakeInfo {
    /// The peer's static public key, proven by the handshake.
    pub remote_public_key: [u8; KEY_LEN],
    /// Noise handshake hash: identical on both ends only if nobody sits in
    /// the middle.
    pub handshake_hash: [u8; 32],
}

impl HandshakeInfo {
    /// Six-digit code both users can compare (like Bluetooth numeric
    /// comparison). A man-in-the-middle runs two separate handshakes, so the
    /// two victims would see different codes.
    pub fn verification_code(&self) -> String {
        let key = blake3::derive_key(VERIFICATION_CODE_CONTEXT, &self.handshake_hash);
        let number = u64::from_le_bytes(key[..8].try_into().expect("8 bytes")) % 1_000_000;
        format!("{number:06}")
    }
}

/// Encrypts each record as one Noise transport message.
pub struct NoiseCipher {
    transport: snow::TransportState,
}

impl RecordCipher for NoiseCipher {
    fn seal(&mut self, plaintext: &[u8], out: &mut Vec<u8>) -> Result<(), CipherError> {
        let start = out.len();
        out.resize(start + plaintext.len() + AEAD_TAG_LEN, 0);
        let written = self
            .transport
            .write_message(plaintext, &mut out[start..])
            .map_err(|e| CipherError(e.to_string()))?;
        out.truncate(start + written);
        Ok(())
    }

    fn open(&mut self, record: &[u8], out: &mut Vec<u8>) -> Result<(), CipherError> {
        let start = out.len();
        out.resize(start + record.len(), 0);
        let written = self
            .transport
            .read_message(record, &mut out[start..])
            .map_err(|_| CipherError("record failed authentication".to_string()))?;
        out.truncate(start + written);
        Ok(())
    }
}

impl From<snow::Error> for ConnectionError {
    fn from(e: snow::Error) -> Self {
        ConnectionError::Handshake(e.to_string())
    }
}

/// Runs a Noise XX handshake on `stream` and returns an encrypted connection.
pub async fn secure_connection<S>(
    stream: S,
    identity: &Identity,
    role: Role,
) -> Result<(Connection<S, NoiseCipher>, HandshakeInfo), ConnectionError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(stream, identity, role))
        .await
        .map_err(|_| ConnectionError::Timeout("Noise handshake"))?
}

async fn handshake<S>(
    mut stream: S,
    identity: &Identity,
    role: Role,
) -> Result<(Connection<S, NoiseCipher>, HandshakeInfo), ConnectionError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let builder = snow::Builder::new(noise_params())
        .local_private_key(identity.private_key())?
        .prologue(PROLOGUE)?;
    let mut state = match role {
        Role::Initiator => builder.build_initiator()?,
        Role::Responder => builder.build_responder()?,
    };

    // XX is three messages: initiator → responder → initiator.
    let mut message = vec![0u8; MAX_RECORD_LEN];
    let mut payload = vec![0u8; MAX_RECORD_LEN];
    let mut our_turn = role == Role::Initiator;
    while !state.is_handshake_finished() {
        if our_turn {
            let len = state.write_message(&[], &mut message)?;
            stream.write_all(&(len as u16).to_be_bytes()).await?;
            stream.write_all(&message[..len]).await?;
            stream.flush().await?;
        } else {
            let len = match stream.read_u16().await {
                Ok(len) => len as usize,
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Err(ConnectionError::Closed)
                }
                Err(e) => return Err(e.into()),
            };
            stream.read_exact(&mut message[..len]).await?;
            state.read_message(&message[..len], &mut payload)?;
        }
        our_turn = !our_turn;
    }

    let remote_public_key = state
        .get_remote_static()
        .and_then(|key| key.try_into().ok())
        .ok_or_else(|| ConnectionError::Handshake("peer sent no static key".to_string()))?;
    let handshake_hash = state
        .get_handshake_hash()
        .try_into()
        .map_err(|_| ConnectionError::Handshake("unexpected handshake hash length".into()))?;
    let transport = state.into_transport_mode()?;

    Ok((
        Connection::new(stream, NoiseCipher { transport }),
        HandshakeInfo {
            remote_public_key,
            handshake_hash,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Frame;
    use crate::messages::Ping;
    use tokio::io::duplex;

    async fn connected_pair(
        a: &Identity,
        b: &Identity,
    ) -> (
        (
            Connection<tokio::io::DuplexStream, NoiseCipher>,
            HandshakeInfo,
        ),
        (
            Connection<tokio::io::DuplexStream, NoiseCipher>,
            HandshakeInfo,
        ),
    ) {
        let (x, y) = duplex(1 << 20);
        let (first, second) = tokio::join!(
            secure_connection(x, a, Role::Initiator),
            secure_connection(y, b, Role::Responder)
        );
        (first.unwrap(), second.unwrap())
    }

    #[tokio::test]
    async fn handshake_authenticates_both_static_keys() {
        let (alice, bob) = (Identity::generate(), Identity::generate());
        let ((mut a, a_info), (mut b, b_info)) = connected_pair(&alice, &bob).await;

        assert_eq!(&a_info.remote_public_key, bob.public_key());
        assert_eq!(&b_info.remote_public_key, alice.public_key());
        assert_eq!(a_info.handshake_hash, b_info.handshake_hash);
        assert_eq!(a_info.verification_code(), b_info.verification_code());
        assert_eq!(a_info.verification_code().len(), 6);

        a.send(&Frame::Ping(Ping { nonce: 7 })).await.unwrap();
        assert_eq!(
            b.read_frame().await.unwrap(),
            Frame::Ping(Ping { nonce: 7 })
        );
        b.send(&Frame::Pong(Ping { nonce: 7 })).await.unwrap();
        assert_eq!(
            a.read_frame().await.unwrap(),
            Frame::Pong(Ping { nonce: 7 })
        );
    }

    #[tokio::test]
    async fn every_session_gets_a_different_code() {
        let (alice, bob) = (Identity::generate(), Identity::generate());
        let ((_, first), _) = connected_pair(&alice, &bob).await;
        let ((_, second), _) = connected_pair(&alice, &bob).await;
        assert_ne!(first.handshake_hash, second.handshake_hash);
    }

    #[tokio::test]
    async fn tampered_record_is_rejected() {
        let (alice, bob) = (Identity::generate(), Identity::generate());
        let ((mut a, _), (mut b, _)) = connected_pair(&alice, &bob).await;

        let mut record = Vec::new();
        a.cipher_mut().seal(b"secret", &mut record).unwrap();
        assert!(
            !record.windows(6).any(|w| w == b"secret"),
            "ciphertext hides it"
        );
        record[0] ^= 1;
        let mut opened = Vec::new();
        assert!(b.cipher_mut().open(&record, &mut opened).is_err());
    }

    #[tokio::test]
    async fn stalled_peer_times_out() {
        tokio::time::pause();
        let (x, _y) = duplex(1024);
        let result = secure_connection(x, &Identity::generate(), Role::Responder).await;
        assert!(matches!(result, Err(ConnectionError::Timeout(_))));
    }
}
