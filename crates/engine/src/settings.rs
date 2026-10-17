//! `settings.toml` loading. Missing files are written with defaults on first run.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("settings io: {0}")]
    Io(#[from] std::io::Error),
    #[error("settings parse: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("settings serialize: {0}")]
    Serialize(#[from] toml::ser::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VsyncMode {
    /// `Fifo`: always available, capped to refresh rate.
    On,
    /// `Mailbox`: uncapped rendering without tearing, where supported.
    Mailbox,
    /// `Immediate`: uncapped, may tear.
    Off,
}

impl VsyncMode {
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::On => Self::Mailbox,
            Self::Mailbox => Self::Off,
            Self::Off => Self::On,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    pub vsync: VsyncMode,
    /// Vertical field of view in degrees.
    pub fov_deg: f32,
    /// Radians of rotation per raw mouse count.
    pub mouse_sensitivity: f32,
    pub seed: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            width: 1600,
            height: 900,
            fullscreen: false,
            vsync: VsyncMode::On,
            fov_deg: 90.0,
            mouse_sensitivity: 0.0022,
            seed: 0,
        }
    }
}

impl Settings {
    /// Reads `path`, or writes and returns the defaults if it does not exist.
    pub fn load_or_create(path: &Path) -> Result<Self, SettingsError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(toml::from_str(&text)?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let defaults = Self::default();
                std::fs::write(path, toml::to_string_pretty(&defaults)?)?;
                Ok(defaults)
            }
            Err(err) => Err(err.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_file_fills_defaults() {
        let s: Settings = toml::from_str("fov_deg = 100.0\nvsync = \"mailbox\"").unwrap();
        assert!((s.fov_deg - 100.0).abs() < 1e-6);
        assert_eq!(s.vsync, VsyncMode::Mailbox);
        assert_eq!(s.width, Settings::default().width);
    }

    #[test]
    fn first_run_writes_defaults_then_reads_edits() {
        let dir = std::env::temp_dir().join(format!("ss-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.toml");
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            Settings::load_or_create(&path).unwrap(),
            Settings::default()
        );
        std::fs::write(&path, "width = 800\n").unwrap();
        assert_eq!(Settings::load_or_create(&path).unwrap().width, 800);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
