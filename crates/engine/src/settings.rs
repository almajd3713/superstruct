//! `settings.toml` loading. Missing files are written with defaults on first run.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::stream::StreamConfig;

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
    /// Draw the ray-marched horizon beyond the far band.
    pub horizon: bool,
    /// March the horizon at half resolution and upsample it with depth awareness.
    pub horizon_half_res: bool,
    /// Band radii in metres; see `StreamConfig`.
    pub band_l2_m: f64,
    pub band_l2_mid_m: f64,
    pub band_l1_full_m: f64,
    pub band_l1_far_m: f64,
}

impl Default for Settings {
    fn default() -> Self {
        let bands = StreamConfig::default();
        Self {
            width: 1600,
            height: 900,
            fullscreen: false,
            vsync: VsyncMode::On,
            fov_deg: 90.0,
            mouse_sensitivity: 0.0022,
            seed: 0,
            horizon: true,
            horizon_half_res: false,
            band_l2_m: bands.l2_radius_m,
            band_l2_mid_m: bands.l2_mid_radius_m,
            band_l1_full_m: bands.l1_full_radius_m,
            band_l1_far_m: bands.l1_far_radius_m,
        }
    }
}

impl Settings {
    /// The band radii as a streaming config, kept ordered (each band reaches at least as far as
    /// the one inside it).
    pub fn stream_config(&self) -> StreamConfig {
        StreamConfig {
            l2_radius_m: self.band_l2_m,
            l2_mid_radius_m: self.band_l2_mid_m,
            l1_full_radius_m: self.band_l1_full_m,
            l1_far_radius_m: self.band_l1_far_m,
        }
        .ordered()
    }

    pub fn set_stream_config(&mut self, cfg: &StreamConfig) {
        self.band_l2_m = cfg.l2_radius_m;
        self.band_l2_mid_m = cfg.l2_mid_radius_m;
        self.band_l1_full_m = cfg.l1_full_radius_m;
        self.band_l1_far_m = cfg.l1_far_radius_m;
    }

    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

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
    fn band_radii_round_trip_and_stay_ordered() {
        let mut s = Settings::default();
        let mut cfg = s.stream_config();
        assert_eq!(cfg, StreamConfig::default());
        // An inconsistent request is pulled into order rather than rejected.
        cfg.l2_radius_m = 2000.0;
        cfg.l2_mid_radius_m = 1000.0;
        s.set_stream_config(&cfg);
        let back = s.stream_config();
        assert!(back.l2_mid_radius_m >= back.l2_radius_m);
        let text = toml::to_string_pretty(&s).unwrap();
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), s);
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
