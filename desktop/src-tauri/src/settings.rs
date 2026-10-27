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
        }
    }
}

impl Settings {
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
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
