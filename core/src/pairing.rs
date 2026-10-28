//! QR-code pairing (SPEC.md section 6.3).
//!
//! A computer shows a `beam://pair` link as a QR code. It carries the
//! computer's id, public key, name and addresses, plus a one-time **pairing
//! token** valid for a few minutes. A phone that scans it:
//!
//! 1. trusts the computer's key (it came straight off the screen);
//! 2. connects, checks the Noise handshake proves *exactly* that key, and sends
//!    `PAIR_REQUEST {token}`;
//! 3. the computer, seeing a valid unused token, trusts the phone's proven key
//!    and answers `PAIR_RESULT`.
//!
//! The token is what lets the computer trust a key it has never seen: only
//! someone who saw the screen has it.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::net::TcpStream;

use crate::connection::ConnectionError;
use crate::discovery::first_reachable;
use crate::frame::Frame;
use crate::messages::{DeviceId, PairRequest, PairResult};
use crate::transfer::{open_control, LocalDevice, TransferError};
use crate::trust::{TrustStore, TrustedDevice};

/// How long a pairing code stays usable after it's shown.
pub const PAIRING_CODE_LIFETIME: Duration = Duration::from_secs(5 * 60);
pub const TOKEN_LEN: usize = 16;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PairingCodeError {
    #[error("not a Beam pairing code")]
    NotBeam,
    #[error("unsupported pairing code version")]
    Version,
    #[error("pairing code is missing or has a bad {0}")]
    Field(&'static str),
    #[error("pairing code's id doesn't match its key")]
    IdMismatch,
}

/// Everything in a pairing QR code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingCode {
    pub device_id: DeviceId,
    pub public_key: [u8; 32],
    pub name: String,
    pub addresses: Vec<SocketAddr>,
    pub token: [u8; TOKEN_LEN],
}

impl PairingCode {
    /// `beam://pair?v=1&id=…&key=…&name=…&addr=…&token=…`
    pub fn to_uri(&self) -> String {
        let addresses: Vec<String> = self.addresses.iter().map(|a| a.to_string()).collect();
        format!(
            "beam://pair?v=1&id={}&key={}&name={}&addr={}&token={}",
            hex::encode(self.device_id.0),
            hex::encode(self.public_key),
            percent_encode(&self.name),
            percent_encode(&addresses.join(",")),
            hex::encode(self.token),
        )
    }

    pub fn parse(uri: &str) -> Result<PairingCode, PairingCodeError> {
        let query = uri
            .strip_prefix("beam://pair?")
            .ok_or(PairingCodeError::NotBeam)?;
        let mut fields = std::collections::HashMap::new();
        for pair in query.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                fields.insert(
                    key,
                    percent_decode(value).ok_or(PairingCodeError::Field(key_name(key)))?,
                );
            }
        }
        if fields.get("v").map(String::as_str) != Some("1") {
            return Err(PairingCodeError::Version);
        }
        let hex_field = |key: &'static str, len: usize| -> Result<Vec<u8>, PairingCodeError> {
            fields
                .get(key)
                .and_then(|v| hex::decode(v).ok())
                .filter(|b| b.len() == len)
                .ok_or(PairingCodeError::Field(key))
        };
        let device_id = DeviceId(hex_field("id", 8)?.try_into().expect("8 bytes"));
        let public_key: [u8; 32] = hex_field("key", 32)?.try_into().expect("32 bytes");
        let token: [u8; TOKEN_LEN] = hex_field("token", TOKEN_LEN)?.try_into().expect("16 bytes");
        if DeviceId::from_public_key(&public_key) != device_id {
            return Err(PairingCodeError::IdMismatch);
        }
        let name = fields
            .get("name")
            .cloned()
            .ok_or(PairingCodeError::Field("name"))?;
        let addresses = fields
            .get("addr")
            .map(|list| {
                list.split(',')
                    .filter(|a| !a.is_empty())
                    .map(|a| a.parse().map_err(|_| PairingCodeError::Field("addr")))
                    .collect::<Result<Vec<SocketAddr>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        Ok(PairingCode {
            device_id,
            public_key,
            name,
            addresses,
            token,
        })
    }
}

fn key_name(key: &str) -> &'static str {
    match key {
        "name" => "name",
        "addr" => "addr",
        _ => "field",
    }
}

/// The receiving side's current pairing token, if a code is on screen.
/// Cheap to clone; every clone shares the same token.
#[derive(Debug, Clone, Default)]
pub struct PairingWindow {
    current: Arc<Mutex<Option<OpenCode>>>,
}

/// The token on screen and when it stops working.
type OpenCode = ([u8; TOKEN_LEN], Instant);

impl PairingWindow {
    /// Starts a new pairing window and returns its token (any earlier
    /// token stops working).
    pub fn open(&self) -> [u8; TOKEN_LEN] {
        let token = *uuid::Uuid::new_v4().as_bytes();
        *self.lock() = Some((token, Instant::now() + PAIRING_CODE_LIFETIME));
        token
    }

    pub fn close(&self) {
        *self.lock() = None;
    }

    /// Uses up `token` if it is the current one and hasn't expired. Each
    /// code pairs one device.
    pub(crate) fn redeem(&self, token: &[u8; TOKEN_LEN]) -> bool {
        let mut current = self.lock();
        match *current {
            Some((expected, until))
                if Instant::now() < until && constant_time_eq(&expected, token) =>
            {
                *current = None;
                true
            }
            _ => false,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<OpenCode>> {
        self.current.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Compares without stopping at the first difference, so the time taken
/// says nothing about how much of a guessed token was right.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The scanning side: connects to the device in `code`, proves we scanned
/// it, and trusts it (with auto-accept). Returns the new trust entry.
pub async fn pair_with(
    code: &PairingCode,
    me: &LocalDevice,
    trust: &TrustStore,
) -> Result<TrustedDevice, TransferError> {
    let addr = first_reachable(&code.addresses).await.ok_or_else(|| {
        TransferError::Connection(ConnectionError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("\"{}\" didn't answer on any of its addresses", code.name),
        )))
    })?;
    let stream = TcpStream::connect(addr)
        .await
        .map_err(ConnectionError::from)?;
    let (mut conn, peer, _) = open_control(stream, me, trust).await?;
    if peer.public_key != code.public_key {
        return Err(TransferError::UnexpectedPeer {
            expected: code.device_id,
            actual: peer.device_id(),
        });
    }
    conn.send(&Frame::PairRequest(PairRequest { token: code.token }))
        .await?;
    match conn.read_frame().await? {
        Frame::PairResult(PairResult { accepted: true, .. }) => {
            let device = TrustedDevice::new(code.name.clone(), code.public_key, true);
            trust.upsert(device.clone())?;
            let _ = conn.shutdown().await;
            Ok(device)
        }
        Frame::PairResult(PairResult { reason, .. }) => Err(TransferError::PairingRefused(reason)),
        other => Err(TransferError::unexpected("PAIR_RESULT", &other)),
    }
}

/// Percent-encodes everything except RFC 3986 unreserved characters.
pub(crate) fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code() -> PairingCode {
        let public_key = [7u8; 32];
        PairingCode {
            device_id: DeviceId::from_public_key(&public_key),
            public_key,
            name: "Kartikay's PC".into(),
            addresses: vec![
                "192.168.1.20:9000".parse().unwrap(),
                "[fe80::1]:9000".parse().unwrap(),
            ],
            token: [9; TOKEN_LEN],
        }
    }

    #[test]
    fn uri_round_trips() {
        let original = code();
        let uri = original.to_uri();
        assert!(uri.starts_with("beam://pair?v=1&id="));
        assert!(uri.contains("&name=Kartikay%27s%20PC"));
        assert_eq!(PairingCode::parse(&uri), Ok(original));
    }

    #[test]
    fn bad_codes_are_rejected() {
        let good = code().to_uri();
        assert_eq!(
            PairingCode::parse("https://example.com"),
            Err(PairingCodeError::NotBeam)
        );
        assert_eq!(
            PairingCode::parse(&good.replace("v=1", "v=2")),
            Err(PairingCodeError::Version)
        );
        assert_eq!(
            PairingCode::parse(&good.replace(&hex::encode([9u8; TOKEN_LEN]), "abcd")),
            Err(PairingCodeError::Field("token"))
        );
        // A key that doesn't hash to the id: someone edited the code.
        let tampered = good.replace(&hex::encode([7u8; 32]), &hex::encode([8u8; 32]));
        assert_eq!(
            PairingCode::parse(&tampered),
            Err(PairingCodeError::IdMismatch)
        );
    }

    #[test]
    fn tokens_are_single_use_and_replaced_by_newer_ones() {
        let window = PairingWindow::default();
        assert!(!window.redeem(&[0; TOKEN_LEN]), "nothing open");
        let first = window.open();
        let second = window.open();
        assert!(!window.redeem(&first), "superseded");
        assert!(window.redeem(&second));
        assert!(!window.redeem(&second), "already used");
    }

    #[test]
    fn tokens_expire() {
        let window = PairingWindow::default();
        let token = window.open();
        // Pretend the code has been on screen too long.
        window.lock().as_mut().unwrap().1 = Instant::now() - Duration::from_secs(1);
        assert!(!window.redeem(&token));
    }
}
