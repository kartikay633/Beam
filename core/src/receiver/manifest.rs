//! Progress file for resume (SPEC.md section 7.3).
//!
//! One JSON manifest per accepted transfer, at
//! `<download dir>/.beam-partial/<transfer id>.json`. It is written as soon
//! as the transfer is accepted (so a reconnect never asks the user again),
//! then at least every 2 seconds, always *after* the written chunks were
//! synced to disk. For each file it records the offered fingerprint (name,
//! size, modified time), the reserved final name, the bitmap of verified
//! chunks, and the whole-file hash once the file is finished.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::sanitize::PART_SUFFIX;

pub(crate) const PARTIAL_DIR: &str = ".beam-partial";
/// Unfinished transfers are kept this long, then cleaned up.
pub(crate) const KEEP_PARTIALS_FOR: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub version: u32,
    pub transfer_id: Uuid,
    /// The sender's static public key (hex): only that device may resume.
    pub sender_key: String,
    pub updated_unix: u64,
    pub files: Vec<ManifestFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ManifestFile {
    pub file_id: u32,
    /// Name as offered (before sanitizing), part of the fingerprint.
    pub name: String,
    pub size: u64,
    pub modified_unix: i64,
    /// Name reserved in the download folder; data goes to `<final_name>.beam-part`.
    pub final_name: String,
    /// Verified and written chunks, hex-encoded bitmap.
    pub chunk_bitmap: String,
    /// Whole-file BLAKE3 (hex) once the file is verified and renamed.
    pub done_blake3: Option<String>,
}

impl Manifest {
    pub(crate) fn path(dir: &Path, transfer_id: Uuid) -> PathBuf {
        dir.join(PARTIAL_DIR).join(format!("{transfer_id}.json"))
    }

    /// Loads the manifest for `transfer_id`, if there is a readable one.
    pub(crate) fn load(dir: &Path, transfer_id: Uuid) -> Option<Manifest> {
        let bytes = std::fs::read(Self::path(dir, transfer_id)).ok()?;
        let manifest: Manifest = serde_json::from_slice(&bytes).ok()?;
        (manifest.version == 1 && manifest.transfer_id == transfer_id).then_some(manifest)
    }

    /// Writes to a temporary file and renames it into place, so a crash never
    /// leaves a half-written manifest.
    pub(crate) fn save(&mut self, dir: &Path) -> io::Result<()> {
        self.updated_unix = now_unix();
        let path = Self::path(dir, self.transfer_id);
        std::fs::create_dir_all(path.parent().expect("manifest has a parent"))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(self).expect("manifest serializes"))?;
        std::fs::rename(&tmp, &path)
    }

    /// Deletes the manifest and the `.beam-part` files of unfinished files.
    /// Finished files are kept.
    pub(crate) fn discard(&self, dir: &Path) {
        for file in self.files.iter().filter(|f| f.done_blake3.is_none()) {
            let _ = std::fs::remove_file(part_path(dir, &file.final_name));
        }
        let _ = std::fs::remove_file(Self::path(dir, self.transfer_id));
    }
}

pub(crate) fn part_path(dir: &Path, final_name: &str) -> PathBuf {
    dir.join(format!("{final_name}{PART_SUFFIX}"))
}

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Removes unfinished transfers older than [`KEEP_PARTIALS_FOR`], except
/// the ones in `active`.
pub(crate) fn remove_stale(dir: &Path, active: &[Uuid]) {
    let Ok(entries) = std::fs::read_dir(dir.join(PARTIAL_DIR)) else {
        return;
    };
    let cutoff = now_unix().saturating_sub(KEEP_PARTIALS_FOR.as_secs());
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(id) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<Uuid>().ok())
        else {
            continue;
        };
        if active.contains(&id) {
            continue;
        }
        match Manifest::load(dir, id) {
            Some(manifest) if manifest.updated_unix < cutoff => manifest.discard(dir),
            Some(_) => {}
            // Unreadable manifest: nothing can resume from it.
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: Uuid) -> Manifest {
        Manifest {
            version: 1,
            transfer_id: id,
            sender_key: "ab".repeat(32),
            updated_unix: 0,
            files: vec![
                ManifestFile {
                    file_id: 0,
                    name: "a.bin".into(),
                    size: 10,
                    modified_unix: 5,
                    final_name: "a.bin".into(),
                    chunk_bitmap: "01".into(),
                    done_blake3: None,
                },
                ManifestFile {
                    file_id: 1,
                    name: "b.bin".into(),
                    size: 10,
                    modified_unix: 5,
                    final_name: "b.bin".into(),
                    chunk_bitmap: "01".into(),
                    done_blake3: Some("cd".repeat(32)),
                },
            ],
        }
    }

    #[test]
    fn save_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        let mut original = manifest(id);
        original.save(dir.path()).unwrap();
        let loaded = Manifest::load(dir.path(), id).unwrap();
        assert_eq!(loaded.files, original.files);
        assert!(loaded.updated_unix > 0);
        assert!(Manifest::load(dir.path(), Uuid::new_v4()).is_none());
    }

    #[test]
    fn discard_keeps_finished_files() {
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        let mut m = manifest(id);
        m.save(dir.path()).unwrap();
        std::fs::write(part_path(dir.path(), "a.bin"), b"partial").unwrap();
        std::fs::write(dir.path().join("b.bin"), b"finished").unwrap();

        m.discard(dir.path());
        assert!(!part_path(dir.path(), "a.bin").exists());
        assert!(dir.path().join("b.bin").exists());
        assert!(Manifest::load(dir.path(), id).is_none());
    }

    #[test]
    fn stale_partials_are_removed_unless_active() {
        let dir = tempfile::tempdir().unwrap();
        let (old, active, fresh) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        for id in [old, active, fresh] {
            let mut m = manifest(id);
            m.files[0].final_name = format!("{id}.bin");
            m.save(dir.path()).unwrap();
            std::fs::write(part_path(dir.path(), &m.files[0].final_name), b"x").unwrap();
            if id != fresh {
                // Backdate by rewriting the timestamp directly.
                m.updated_unix = 1;
                let path = Manifest::path(dir.path(), id);
                std::fs::write(path, serde_json::to_vec(&m).unwrap()).unwrap();
            }
        }
        remove_stale(dir.path(), &[active]);
        assert!(Manifest::load(dir.path(), old).is_none());
        assert!(!part_path(dir.path(), &format!("{old}.bin")).exists());
        assert!(Manifest::load(dir.path(), active).is_some());
        assert!(Manifest::load(dir.path(), fresh).is_some());
    }
}
