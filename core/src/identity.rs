//! Device identity (SPEC.md section 6.1): a static X25519 key pair, generated
//! once and reused as the Noise static key. The device id is the first 8
//! bytes of SHA-256(public key).
//!
//! The CLI keeps the key in a JSON file in its config folder. The desktop and
//! Android apps will move it to DPAPI / Android Keystore in later phases.

use std::fmt;
use std::io::{self, Write};
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::messages::DeviceId;
use crate::secure::noise_params;

pub const KEY_LEN: usize = 32;

#[derive(Clone)]
pub struct Identity {
    private_key: [u8; KEY_LEN],
    public_key: [u8; KEY_LEN],
}

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("could not read or write the identity file {path}: {source}")]
    Io { path: String, source: io::Error },
    #[error("identity file {path} is corrupt: {reason}")]
    Corrupt { path: String, reason: String },
}

/// On-disk format of the identity file.
#[derive(Serialize, Deserialize)]
struct StoredIdentity {
    version: u32,
    private_key: String,
    public_key: String,
}

impl Identity {
    /// Generates a fresh random key pair.
    pub fn generate() -> Identity {
        let keypair = snow::Builder::new(noise_params())
            .generate_keypair()
            .expect("X25519 key generation with the default resolver cannot fail");
        Identity {
            private_key: keypair.private.try_into().expect("32-byte X25519 key"),
            public_key: keypair.public.try_into().expect("32-byte X25519 key"),
        }
    }

    /// Loads the identity stored at `path`, or creates and saves a new one if
    /// the file doesn't exist yet.
    pub fn load_or_create(path: &Path) -> Result<Identity, IdentityError> {
        match std::fs::read(path) {
            Ok(bytes) => Self::parse(&bytes).map_err(|reason| IdentityError::Corrupt {
                path: path.display().to_string(),
                reason,
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let identity = Identity::generate();
                identity.save(path)?;
                Ok(identity)
            }
            Err(source) => Err(IdentityError::Io {
                path: path.display().to_string(),
                source,
            }),
        }
    }

    fn parse(bytes: &[u8]) -> Result<Identity, String> {
        let stored: StoredIdentity = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if stored.version != 1 {
            return Err(format!("unsupported version {}", stored.version));
        }
        Ok(Identity {
            private_key: decode_key(&stored.private_key)?,
            public_key: decode_key(&stored.public_key)?,
        })
    }

    fn save(&self, path: &Path) -> Result<(), IdentityError> {
        let io_err = |source| IdentityError::Io {
            path: path.display().to_string(),
            source,
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io_err)?;
        }
        let stored = StoredIdentity {
            version: 1,
            private_key: hex::encode(self.private_key),
            public_key: hex::encode(self.public_key),
        };
        let json = serde_json::to_vec_pretty(&stored).expect("identity serializes");

        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            // Only the owner may read the private key.
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(io_err)?;
        file.write_all(&json).map_err(io_err)?;
        file.sync_all().map_err(io_err)
    }

    /// The private and public key, for storing in a platform keystore
    /// (DPAPI, Android Keystore). Treat the result as a secret.
    pub fn export_secret(&self) -> [u8; 2 * KEY_LEN] {
        let mut out = [0u8; 2 * KEY_LEN];
        out[..KEY_LEN].copy_from_slice(&self.private_key);
        out[KEY_LEN..].copy_from_slice(&self.public_key);
        out
    }

    /// Rebuilds an identity from [`Identity::export_secret`] bytes.
    pub fn import_secret(bytes: &[u8]) -> Option<Identity> {
        if bytes.len() != 2 * KEY_LEN {
            return None;
        }
        Some(Identity {
            private_key: bytes[..KEY_LEN].try_into().ok()?,
            public_key: bytes[KEY_LEN..].try_into().ok()?,
        })
    }

    pub fn public_key(&self) -> &[u8; KEY_LEN] {
        &self.public_key
    }

    pub(crate) fn private_key(&self) -> &[u8; KEY_LEN] {
        &self.private_key
    }

    pub fn device_id(&self) -> DeviceId {
        DeviceId::from_public_key(&self.public_key)
    }
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print the private key.
        f.debug_struct("Identity")
            .field("device_id", &self.device_id().to_string())
            .finish_non_exhaustive()
    }
}

impl DeviceId {
    /// First 8 bytes of SHA-256 of a static public key.
    pub fn from_public_key(public_key: &[u8]) -> DeviceId {
        let digest = Sha256::digest(public_key);
        let mut id = [0u8; 8];
        id.copy_from_slice(&digest[..8]);
        DeviceId(id)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("a device id is 16 hex digits, like 0123-4567-89ab-cdef")]
pub struct ParseDeviceIdError;

impl FromStr for DeviceId {
    type Err = ParseDeviceIdError;

    /// Accepts `0123-4567-89ab-cdef` or `0123456789abcdef`, any case.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let digits: String = s.chars().filter(|&c| c != '-').collect();
        let bytes = hex::decode(digits).map_err(|_| ParseDeviceIdError)?;
        Ok(DeviceId(bytes.try_into().map_err(|_| ParseDeviceIdError)?))
    }
}

pub(crate) fn decode_key(hex_key: &str) -> Result<[u8; KEY_LEN], String> {
    hex::decode(hex_key)
        .map_err(|e| format!("bad hex: {e}"))?
        .try_into()
        .map_err(|_| "a key must be 32 bytes".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_are_random_and_ids_follow_the_key() {
        let a = Identity::generate();
        let b = Identity::generate();
        assert_ne!(a.public_key(), b.public_key());
        assert_ne!(a.device_id(), b.device_id());
        assert_eq!(a.device_id(), DeviceId::from_public_key(a.public_key()));
    }

    #[test]
    fn device_id_is_the_sha256_prefix_of_the_public_key() {
        // SHA-256 of 32 zero bytes starts with 66687aadf862bd77.
        let id = DeviceId::from_public_key(&[0u8; 32]);
        assert_eq!(id.to_string(), "6668-7aad-f862-bd77");
    }

    #[test]
    fn identity_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("identity.json");
        let first = Identity::load_or_create(&path).unwrap();
        let second = Identity::load_or_create(&path).unwrap();
        assert_eq!(first.public_key(), second.public_key());
        assert_eq!(first.private_key(), second.private_key());
    }

    #[test]
    fn corrupt_identity_file_is_reported_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.json");
        std::fs::write(&path, b"{not json").unwrap();
        assert!(matches!(
            Identity::load_or_create(&path),
            Err(IdentityError::Corrupt { .. })
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"{not json");
    }

    #[test]
    fn secret_export_round_trips() {
        let identity = Identity::generate();
        let copy = Identity::import_secret(&identity.export_secret()).unwrap();
        assert_eq!(copy.public_key(), identity.public_key());
        assert_eq!(copy.private_key(), identity.private_key());
        assert!(Identity::import_secret(&[0; 10]).is_none());
    }

    #[test]
    fn debug_output_hides_the_private_key() {
        let identity = Identity::generate();
        let debug = format!("{identity:?}");
        assert!(!debug.contains(&hex::encode(identity.private_key())));
    }

    #[test]
    fn device_ids_parse_with_or_without_dashes() {
        let id = DeviceId([0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]);
        assert_eq!("0123-4567-89ab-cdef".parse(), Ok(id));
        assert_eq!("0123456789ABCDEF".parse(), Ok(id));
        assert_eq!("0123".parse::<DeviceId>(), Err(ParseDeviceIdError));
        assert_eq!(
            "zz23456789abcdef".parse::<DeviceId>(),
            Err(ParseDeviceIdError)
        );
    }
}
