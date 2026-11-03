//! User settings, stored as JSON in the app's config folder.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Shown to other devices. Takes effect after a restart.
    pub device_name: String,
    pub download_dir: PathBuf,
    /// Parallel data connections when sending.
    pub streams: usize,
    /// Visibility when the app starts: "everyone", "trusted" or "off".
    pub visibility: String,
    /// Keep a "Beam" entry in Explorer's Send To menu.
    pub send_to_shortcut: bool,
    pub start_with_windows: bool,
    /// The first-run question about staying ready in the background has
    /// been answered (either way), so it isn't asked again.
    pub asked_background: bool,
    /// Folders trusted devices may choose to save into, by id: "downloads",
    /// "desktop", "documents", "pictures", "videos". The Beam folder is
    /// always allowed.
    pub save_folders: Vec<String>,
    /// Any other folders trusted devices may save into, offered by their
    /// name as `pin:<name>` (for example `pin:Projects`).
    pub pinned_folders: Vec<PathBuf>,
}

/// Folder ids for pinned folders start with this; the rest is the name shown.
pub const PINNED_PREFIX: &str = "pin:";

/// The folders a phone can pick, with their Windows locations.
pub const SAVE_FOLDER_IDS: [&str; 5] = ["downloads", "desktop", "documents", "pictures", "videos"];

/// Where folder `id` is on this PC.
pub fn save_folder_path(id: &str) -> Option<PathBuf> {
    match id {
        "downloads" => dirs::download_dir(),
        "desktop" => dirs::desktop_dir(),
        "documents" => dirs::document_dir(),
        "pictures" => dirs::picture_dir(),
        "videos" => dirs::video_dir(),
        _ => None,
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            device_name: std::env::var("COMPUTERNAME")
                .or_else(|_| std::env::var("HOSTNAME"))
                .unwrap_or_else(|_| "Beam desktop".to_string()),
            download_dir: dirs::download_dir()
                .or_else(dirs::home_dir)
                .unwrap_or_default()
                .join("Beam"),
            streams: 4,
            visibility: "everyone".to_string(),
            send_to_shortcut: true,
            start_with_windows: false,
            asked_background: false,
            save_folders: SAVE_FOLDER_IDS.iter().map(|id| id.to_string()).collect(),
            pinned_folders: Vec::new(),
        }
    }
}

impl Settings {
    /// The allowed folders that exist on this PC, for the receiver.
    pub fn save_folder_paths(&self) -> Vec<(String, PathBuf)> {
        let mut folders: Vec<(String, PathBuf)> = self
            .save_folders
            .iter()
            .filter_map(|id| Some((id.clone(), save_folder_path(id)?)))
            .collect();
        for dir in self.pinned_folders.iter().filter(|dir| dir.is_dir()) {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| dir.display().to_string());
            // Two pinned folders with the same name: "Photos", "Photos 2".
            let mut id = format!("{PINNED_PREFIX}{name}");
            let mut n = 2;
            while folders.iter().any(|(known, _)| known == &id) {
                id = format!("{PINNED_PREFIX}{name} {n}");
                n += 1;
            }
            folders.push((id, dir.clone()));
        }
        folders
    }

    /// Loads settings; a missing or unreadable file gives the defaults.
    pub fn load(path: &Path) -> Settings {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
            .map(Settings::sanitized)
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).expect("settings serialize"),
        )?;
        std::fs::rename(&tmp, path)
    }

    /// Commit only after both operating-system integrations succeed. Restore
    /// attempted changes if an integration or the atomic settings save fails.
    pub fn apply(
        &self,
        old: &Settings,
        path: &Path,
        mut shortcut: impl FnMut(bool) -> Result<(), String>,
        mut autostart: impl FnMut(bool) -> Result<(), String>,
    ) -> Result<(), String> {
        let (mut shortcut_attempted, mut autostart_attempted) = (false, false);
        let result = (|| {
            if self.send_to_shortcut != old.send_to_shortcut {
                shortcut_attempted = true;
                shortcut(self.send_to_shortcut)?;
            }
            if self.start_with_windows != old.start_with_windows {
                autostart_attempted = true;
                autostart(self.start_with_windows)?;
            }
            self.save(path)
                .map_err(|e| format!("could not save settings: {e}"))
        })();
        if let Err(mut error) = result {
            if autostart_attempted {
                if let Err(restore) = autostart(old.start_with_windows) {
                    error.push_str(&format!("; could not restore startup setting: {restore}"));
                }
            }
            if shortcut_attempted {
                if let Err(restore) = shortcut(old.send_to_shortcut) {
                    error.push_str(&format!("; could not restore Send To shortcut: {restore}"));
                }
            }
            return Err(error);
        }
        Ok(())
    }

    /// Clamps values a hand-edited file could get wrong.
    pub fn sanitized(mut self) -> Settings {
        self.streams = self.streams.clamp(1, 16);
        if self.device_name.trim().is_empty() {
            self.device_name = Settings::default().device_name;
        }
        if self.download_dir.as_os_str().is_empty() {
            self.download_dir = Settings::default().download_dir;
        }
        if !matches!(self.visibility.as_str(), "everyone" | "trusted" | "off") {
            self.visibility = "everyone".to_string();
        }
        self.save_folders
            .retain(|id| SAVE_FOLDER_IDS.contains(&id.as_str()));
        self.save_folders.dedup();
        self.pinned_folders.retain(|dir| dir.is_absolute());
        self.pinned_folders.dedup();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_folders_are_offered_by_name_and_kept_apart() {
        let tmp = std::env::temp_dir().join(format!("beam-pinned-{}", std::process::id()));
        let (a, b) = (
            tmp.join("one").join("Photos"),
            tmp.join("two").join("Photos"),
        );
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let settings = Settings {
            save_folders: vec![],
            pinned_folders: vec![a.clone(), b.clone(), tmp.join("missing")],
            ..Settings::default()
        };
        assert_eq!(
            settings.save_folder_paths(),
            vec![
                ("pin:Photos".to_string(), a),
                ("pin:Photos 2".to_string(), b)
            ]
        );
        std::fs::remove_dir_all(tmp).unwrap();
    }

    #[test]
    fn relative_pinned_folders_are_dropped() {
        let settings = Settings {
            pinned_folders: vec![PathBuf::from("relative"), PathBuf::from(r"C:\Work")],
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(settings.pinned_folders, vec![PathBuf::from(r"C:\Work")]);
    }

    #[test]
    fn settings_round_trip_and_default_when_missing() {
        let dir = std::env::temp_dir().join(format!("beam-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        assert_eq!(Settings::load(&path), Settings::default());

        let changed = Settings {
            streams: 8,
            visibility: "trusted".into(),
            ..Settings::default()
        };
        changed.save(&path).unwrap();
        assert_eq!(Settings::load(&path), changed);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bad_values_are_repaired() {
        let repaired = Settings {
            streams: 99,
            visibility: "public".into(),
            device_name: "  ".into(),
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(repaired.streams, 16);
        assert_eq!(repaired.visibility, "everyone");
        assert!(!repaired.device_name.trim().is_empty());
    }

    #[test]
    fn unknown_or_missing_fields_fall_back_to_defaults() {
        let parsed: Settings = serde_json::from_str(r#"{"streams": 2, "future": true}"#).unwrap();
        assert_eq!(parsed.streams, 2);
        assert_eq!(parsed.visibility, "everyone");
    }

    #[test]
    fn failed_integration_preserves_settings_and_restores_the_shortcut() {
        let dir =
            std::env::temp_dir().join(format!("beam-settings-failure-{}", std::process::id()));
        let path = dir.join("settings.json");
        let old = Settings::default();
        old.save(&path).unwrap();
        let new = Settings {
            send_to_shortcut: false,
            start_with_windows: true,
            ..old.clone()
        };
        let mut shortcut = true;
        let result = new.apply(
            &old,
            &path,
            |on| {
                shortcut = on;
                Ok(())
            },
            |on| {
                if on {
                    Err("autostart unavailable".into())
                } else {
                    Ok(())
                }
            },
        );
        assert!(result.unwrap_err().contains("autostart unavailable"));
        assert_eq!(Settings::load(&path), old);
        assert!(shortcut);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_persistence_restores_operating_system_integrations() {
        let dir = std::env::temp_dir().join(format!(
            "beam-settings-write-failure-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // A directory at the destination makes the final atomic rename fail.
        let old = Settings::default();
        let new = Settings {
            send_to_shortcut: false,
            start_with_windows: true,
            ..old.clone()
        };
        let (mut shortcut, mut autostart) = (true, false);
        assert!(new
            .apply(
                &old,
                &dir,
                |on| {
                    shortcut = on;
                    Ok(())
                },
                |on| {
                    autostart = on;
                    Ok(())
                }
            )
            .is_err());
        assert!(shortcut);
        assert!(!autostart);
        let _ = std::fs::remove_file(dir.with_extension("json.tmp"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
