//! Trusted devices (SPEC.md section 6.3): a small JSON file of
//! `{device_id, name, public_key, auto_accept}` records.
//!
//! [`TrustStore`] is a cheap, cloneable handle, so a front-end can keep one
//! copy and add devices while a receiver holds another.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::identity::{decode_key, KEY_LEN};
use crate::messages::DeviceId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedDevice {
    pub device_id: DeviceId,
    pub name: String,
    pub public_key: [u8; KEY_LEN],
    /// Accept transfers from this device without asking.
    pub auto_accept: bool,
}

impl TrustedDevice {
    pub fn new(name: String, public_key: [u8; KEY_LEN], auto_accept: bool) -> TrustedDevice {
        TrustedDevice {
            device_id: DeviceId::from_public_key(&public_key),
            name,
            public_key,
            auto_accept,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TrustStoreError {
    #[error("could not read or write the trust store {path}: {source}")]
    Io { path: String, source: io::Error },
    #[error("trust store {path} is corrupt: {reason}")]
    Corrupt { path: String, reason: String },
}

#[derive(Clone, Debug, Default)]
pub struct TrustStore {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Debug, Default)]
struct Inner {
    /// `None` for an in-memory store (tests).
    path: Option<PathBuf>,
    devices: Vec<TrustedDevice>,
}

/// On-disk format: ids and keys as hex strings so the file is readable.
#[derive(Serialize, Deserialize)]
struct StoredDevice {
    device_id: String,
    name: String,
    public_key: String,
    auto_accept: bool,
}

#[derive(Serialize, Deserialize)]
struct StoredFile {
    version: u32,
    devices: Vec<StoredDevice>,
}

impl TrustStore {
    /// A store that is never written to disk.
    pub fn in_memory() -> TrustStore {
        TrustStore::default()
    }

    /// Opens the store at `path`. A missing file is an empty store; it is
    /// created on the first change.
    pub fn open(path: &Path) -> Result<TrustStore, TrustStoreError> {
        let mut inner = Inner {
            path: Some(path.to_path_buf()),
            devices: Vec::new(),
        };
        inner.reload()?;
        Ok(TrustStore {
            inner: Arc::new(Mutex::new(inner)),
        })
    }

    /// Looks a device up. The file is re-read every time, so a device removed
    /// by another process (say, `beam-cli trust remove` while a receiver
    /// runs) stops being trusted immediately. If the file can't be read, this
    /// fails rather than trusting from a stale copy.
    pub fn get(&self, device_id: &DeviceId) -> Result<Option<TrustedDevice>, TrustStoreError> {
        let mut inner = self.lock();
        inner.reload()?;
        Ok(inner
            .devices
            .iter()
            .find(|d| &d.device_id == device_id)
            .cloned())
    }

    pub fn list(&self) -> Result<Vec<TrustedDevice>, TrustStoreError> {
        let mut inner = self.lock();
        inner.reload()?;
        Ok(inner.devices.clone())
    }

    /// Adds a device, or replaces the entry with the same id.
    pub fn upsert(&self, device: TrustedDevice) -> Result<(), TrustStoreError> {
        let mut inner = self.lock();
        inner.reload()?;
        match inner
            .devices
            .iter_mut()
            .find(|d| d.device_id == device.device_id)
        {
            Some(existing) => *existing = device,
            None => inner.devices.push(device),
        }
        inner.save()
    }

    /// Removes a device. Returns whether it was there.
    pub fn remove(&self, device_id: &DeviceId) -> Result<bool, TrustStoreError> {
        let mut inner = self.lock();
        inner.reload()?;
        let before = inner.devices.len();
        inner.devices.retain(|d| &d.device_id != device_id);
        if inner.devices.len() == before {
            return Ok(false);
        }
        inner.save()?;
        Ok(true)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock can't leave the Vec half-updated in a
        // way that matters here, so recover from poisoning.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Inner {
    /// Replaces the in-memory list with the file's contents. A missing file
    /// means no trusted devices.
    fn reload(&mut self) -> Result<(), TrustStoreError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        self.devices = match std::fs::read(path) {
            Ok(bytes) => parse(&bytes).map_err(|reason| TrustStoreError::Corrupt {
                path: path.display().to_string(),
                reason,
            })?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(source) => {
                return Err(TrustStoreError::Io {
                    path: path.display().to_string(),
                    source,
                })
            }
        };
        Ok(())
    }

    /// Writes to a temporary file and renames it over the real one, so a
    /// crash mid-write never leaves a corrupt store.
    fn save(&self) -> Result<(), TrustStoreError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let io_err = |source| TrustStoreError::Io {
            path: path.display().to_string(),
            source,
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io_err)?;
        }
        let file = StoredFile {
            version: 1,
            devices: self
                .devices
                .iter()
                .map(|d| StoredDevice {
                    device_id: d.device_id.to_string(),
                    name: d.name.clone(),
                    public_key: hex::encode(d.public_key),
                    auto_accept: d.auto_accept,
                })
                .collect(),
        };
        let json = serde_json::to_vec_pretty(&file).expect("trust store serializes");
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(io_err)?;
        std::fs::rename(&tmp, path).map_err(io_err)
    }
}

fn parse(bytes: &[u8]) -> Result<Vec<TrustedDevice>, String> {
    let file: StoredFile = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if file.version != 1 {
        return Err(format!("unsupported version {}", file.version));
    }
    file.devices
        .into_iter()
        .map(|stored| {
            let public_key = decode_key(&stored.public_key)?;
            let device = TrustedDevice::new(stored.name, public_key, stored.auto_accept);
            // The id is derived from the key; a mismatch means the file was edited.
            if stored.device_id.parse() != Ok(device.device_id) {
                return Err(format!(
                    "device id {} does not match its public key",
                    stored.device_id
                ));
            }
            Ok(device)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str, key_byte: u8, auto_accept: bool) -> TrustedDevice {
        TrustedDevice::new(name.to_string(), [key_byte; 32], auto_accept)
    }

    #[test]
    fn changes_are_saved_and_reloaded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trusted_devices.json");

        let store = TrustStore::open(&path).unwrap();
        assert!(store.list().unwrap().is_empty());
        let phone = device("Pixel", 1, true);
        let laptop = device("ThinkPad", 2, false);
        store.upsert(phone.clone()).unwrap();
        store.upsert(laptop.clone()).unwrap();

        let reloaded = TrustStore::open(&path).unwrap();
        assert_eq!(reloaded.list().unwrap(), [phone.clone(), laptop.clone()]);
        assert_eq!(
            reloaded.get(&laptop.device_id).unwrap(),
            Some(laptop.clone())
        );

        assert!(reloaded.remove(&phone.device_id).unwrap());
        assert!(!reloaded.remove(&phone.device_id).unwrap());
        assert_eq!(TrustStore::open(&path).unwrap().list().unwrap(), [laptop]);
    }

    /// Two handles opened separately behave like two processes (a running
    /// receiver and `beam-cli trust remove`).
    #[test]
    fn removal_by_another_process_takes_effect_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trusted_devices.json");
        let receiver_view = TrustStore::open(&path).unwrap();
        let cli_view = TrustStore::open(&path).unwrap();

        let phone = device("Pixel", 1, true);
        cli_view.upsert(phone.clone()).unwrap();
        assert_eq!(
            receiver_view.get(&phone.device_id).unwrap(),
            Some(phone.clone())
        );

        cli_view.remove(&phone.device_id).unwrap();
        assert_eq!(receiver_view.get(&phone.device_id).unwrap(), None);

        // An upsert through a stale handle must not resurrect removed devices.
        cli_view.upsert(phone.clone()).unwrap();
        receiver_view.remove(&phone.device_id).unwrap();
        receiver_view.upsert(device("Other", 2, false)).unwrap();
        assert_eq!(cli_view.get(&phone.device_id).unwrap(), None);
    }

    #[test]
    fn unreadable_store_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trusted_devices.json");
        let store = TrustStore::open(&path).unwrap();
        let phone = device("Pixel", 1, true);
        store.upsert(phone.clone()).unwrap();

        std::fs::write(&path, b"garbage").unwrap();
        assert!(store.get(&phone.device_id).is_err());
    }

    #[test]
    fn upsert_replaces_an_existing_entry() {
        let store = TrustStore::in_memory();
        store.upsert(device("Old name", 1, false)).unwrap();
        store.upsert(device("New name", 1, true)).unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "New name");
        assert!(list[0].auto_accept);
    }

    #[test]
    fn clones_share_the_same_devices() {
        let store = TrustStore::in_memory();
        let other = store.clone();
        other.upsert(device("Pixel", 1, false)).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn edited_id_that_does_not_match_the_key_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trusted_devices.json");
        let store = TrustStore::open(&path).unwrap();
        let pixel = device("Pixel", 1, false);
        store.upsert(pixel.clone()).unwrap();

        let json = std::fs::read_to_string(&path).unwrap();
        let tampered = json.replace(&pixel.device_id.to_string(), "0000-0000-0000-0000");
        std::fs::write(&path, tampered).unwrap();
        assert!(matches!(
            TrustStore::open(&path),
            Err(TrustStoreError::Corrupt { .. })
        ));
    }
}
