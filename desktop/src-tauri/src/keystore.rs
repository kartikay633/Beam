//! Where the device's private key lives.
//!
//! On Windows it's encrypted with DPAPI (`CryptProtectData`), which ties it
//! to the Windows user account: another user, or the file copied to another
//! machine, can't decrypt it. Elsewhere (Linux/macOS builds, LAN-only) it
//! falls back to beam-core's JSON file.

use std::path::Path;

use beam_core::Identity;

/// Loads this device's identity, creating and storing a new one on first run.
pub fn load_or_create(config_dir: &Path) -> Result<Identity, String> {
    #[cfg(windows)]
    {
        let path = config_dir.join("identity.dpapi");
        match std::fs::read(&path) {
            Ok(blob) => {
                let secret = dpapi::unprotect(&blob)
                    .map_err(|e| format!("could not decrypt {}: {e}", path.display()))?;
                Identity::import_secret(&secret)
                    .ok_or_else(|| format!("{} is corrupt", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let identity = Identity::generate();
                let blob = dpapi::protect(&identity.export_secret())
                    .map_err(|e| format!("could not encrypt the device key: {e}"))?;
                std::fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
                std::fs::write(&path, blob)
                    .map_err(|e| format!("could not save {}: {e}", path.display()))?;
                Ok(identity)
            }
            Err(e) => Err(format!("could not read {}: {e}", path.display())),
        }
    }
    #[cfg(not(windows))]
    {
        Identity::load_or_create(&config_dir.join("identity.json")).map_err(|e| e.to_string())
    }
}

#[cfg(windows)]
mod dpapi {
    use std::io;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    /// Extra input mixed into the encryption, so other apps running as the
    /// same user can't simply ask DPAPI to decrypt our blob.
    const ENTROPY: &[u8] = b"beam device identity v1";

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    /// Copies DPAPI's output and frees it.
    ///
    /// # Safety
    /// `out` must have been filled in by a successful DPAPI call.
    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        unsafe { LocalFree(Some(HLOCAL(out.pbData.cast()))) };
        bytes
    }

    pub fn protect(data: &[u8]) -> io::Result<Vec<u8>> {
        let (input, entropy) = (blob(data), blob(ENTROPY));
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
            .map_err(io::Error::other)?;
            Ok(take(out))
        }
    }

    pub fn unprotect(data: &[u8]) -> io::Result<Vec<u8>> {
        let (input, entropy) = (blob(data), blob(ENTROPY));
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptUnprotectData(
                &input,
                None,
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
            .map_err(io::Error::other)?;
            Ok(take(out))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn round_trip_and_tamper_detection() {
            let secret = b"0123456789abcdef0123456789abcdef";
            let blob = protect(secret).unwrap();
            assert!(
                !blob.windows(secret.len()).any(|w| w == secret),
                "encrypted"
            );
            assert_eq!(unprotect(&blob).unwrap(), secret);

            let mut tampered = blob.clone();
            let last = tampered.len() - 1;
            tampered[last] ^= 1;
            assert!(unprotect(&tampered).is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("beam-keystore-{}", std::process::id()));
        let first = load_or_create(&dir).unwrap();
        let second = load_or_create(&dir).unwrap();
        assert_eq!(first.public_key(), second.public_key());
        let _ = std::fs::remove_dir_all(dir);
    }
}
