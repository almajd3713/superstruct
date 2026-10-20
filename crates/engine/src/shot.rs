//! Screenshot sidecars (PW.4.6): every screenshot is saved beside a TOML file with the pose and
//! settings that made it, and `SUPERSTRUCT_SHOT=<sidecar>` starts the game from one, so any
//! report can be replayed.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::settings::Settings;
use crate::streamer::StreamStats;

#[derive(Debug, thiserror::Error)]
pub enum ShotError {
    #[error("shot io: {0}")]
    Io(#[from] std::io::Error),
    #[error("shot parse: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("shot serialize: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// Band radii in metres, as `StreamConfig` has them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bands {
    pub l3: f64,
    pub l2: f64,
    pub l2_mid: f64,
    pub l1_full: f64,
    pub l1_far: f64,
}

/// What the overlay showed when the shot was taken. Only for reading; replay ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Readings {
    pub frame_ms: f64,
    pub worst_ms: f64,
    pub chunks_resident: usize,
    pub chunks_wanted: usize,
    pub queued: usize,
    pub gpu_mib: f64,
    pub evictions: u64,
    pub rejected: u64,
    pub over_budget_frames: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shot {
    pub seed: u64,
    pub gen_version: u32,
    pub sector: [i64; 3],
    /// The camera (eye) inside the sector, metres.
    pub local: [f32; 3],
    /// Radians, as the camera holds them, so a replay is exact. Yaw 0 looks along -z.
    pub yaw: f32,
    pub pitch: f32,
    pub fov_deg: f32,
    pub fly: bool,
    pub width: u32,
    pub height: u32,
    pub bands: Bands,
    #[serde(default)]
    pub readings: Readings,
}

impl Shot {
    /// The settings a replay starts with: `base` with the shot's seed, window, view and bands.
    pub fn apply_to(&self, base: &Settings) -> Settings {
        let mut s = base.clone();
        s.seed = self.seed;
        s.width = self.width;
        s.height = self.height;
        s.fov_deg = self.fov_deg;
        s.band_l3_m = self.bands.l3;
        s.band_l2_m = self.bands.l2;
        s.band_l2_mid_m = self.bands.l2_mid;
        s.band_l1_full_m = self.bands.l1_full;
        s.band_l1_far_m = self.bands.l1_far;
        s
    }

    pub fn load(path: &Path) -> Result<Self, ShotError> {
        Ok(toml::from_str(&std::fs::read_to_string(path)?)?)
    }

    pub fn save(&self, path: &Path) -> Result<(), ShotError> {
        std::fs::write(path, toml::to_string(self)?)?;
        Ok(())
    }
}

impl Bands {
    pub fn of(s: &Settings) -> Self {
        Self {
            l3: s.band_l3_m,
            l2: s.band_l2_m,
            l2_mid: s.band_l2_mid_m,
            l1_full: s.band_l1_full_m,
            l1_far: s.band_l1_far_m,
        }
    }
}

impl Readings {
    pub fn of(stats: &StreamStats, gpu_bytes: u64, frame_ms: f64, worst_ms: f64) -> Self {
        Self {
            frame_ms,
            worst_ms,
            chunks_resident: stats.resident,
            chunks_wanted: stats.wanted,
            queued: stats.pool.queued + stats.pool.running + stats.ready,
            gpu_mib: gpu_bytes as f64 / f64::from(1u32 << 20),
            evictions: stats.evictions,
            rejected: stats.rejected,
            over_budget_frames: stats.over_budget_frames,
        }
    }
}

/// The shot to replay, from `SUPERSTRUCT_SHOT`. An unreadable file is logged and ignored.
pub fn from_env() -> Option<Shot> {
    let path = std::env::var_os("SUPERSTRUCT_SHOT")?;
    match Shot::load(Path::new(&path)) {
        Ok(shot) => {
            if shot.gen_version != worldgen::GEN_VERSION {
                tracing::warn!(
                    shot = shot.gen_version,
                    now = worldgen::GEN_VERSION,
                    "the shot was taken with another generator version; the world may differ"
                );
            }
            Some(shot)
        }
        Err(err) => {
            tracing::error!(path = ?path, "cannot replay shot: {err}");
            None
        }
    }
}

/// Where the next screenshot goes: `out/<SUPERSTRUCT_SHOT_NAME>.png` when that is set, else
/// `out/screenshot-<unix seconds>.png`, with `-2`, `-3`… if that file exists already.
pub fn next_path() -> PathBuf {
    let stem = std::env::var("SUPERSTRUCT_SHOT_NAME").unwrap_or_else(|_| {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        format!("screenshot-{stamp}")
    });
    let mut path = PathBuf::from(format!("out/{stem}.png"));
    let mut n = 2;
    while path.exists() && std::env::var_os("SUPERSTRUCT_SHOT_NAME").is_none() {
        path = PathBuf::from(format!("out/{stem}-{n}.png"));
        n += 1;
    }
    path
}

/// The sidecar beside a screenshot: the same name with `.toml`.
pub fn sidecar_of(png: &Path) -> PathBuf {
    png.with_extension("toml")
}

/// When `SUPERSTRUCT_AUTOSHOT` takes its shot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoShot {
    /// After this many frames.
    After(u64),
    /// Once streaming has been idle for `QUIET_FRAMES` frames in a row, or after `limit`
    /// frames at most (`settle` or `settle:<limit>`).
    Settled { limit: u64 },
}

/// Frames in a row with nothing queued, running or waiting for upload.
pub const QUIET_FRAMES: u32 = 60;
const SETTLE_LIMIT: u64 = 20_000;

impl AutoShot {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if let Some(rest) = text.strip_prefix("settle") {
            let limit = match rest.strip_prefix(':') {
                Some(n) => n.parse().ok()?,
                None if rest.is_empty() => SETTLE_LIMIT,
                None => return None,
            };
            return Some(Self::Settled { limit });
        }
        text.parse().ok().map(Self::After)
    }

    /// Whether to take the shot this frame, given the frame number and how many frames in a
    /// row streaming has been idle.
    pub fn due(self, frame: u64, quiet: u32) -> bool {
        match self {
            Self::After(n) => frame == n,
            Self::Settled { limit } => quiet == QUIET_FRAMES || frame == limit,
        }
    }
}

/// Whether the streamer has nothing left to do (the wanted set may still exceed the memory
/// budget, so resident and wanted are not compared).
pub fn idle(stats: &StreamStats) -> bool {
    stats.pool.queued == 0 && stats.pool.running == 0 && stats.ready == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot() -> Shot {
        Shot {
            seed: 0,
            gen_version: worldgen::GEN_VERSION,
            sector: [-8, 0, -13],
            local: [943.0, 193.0, 978.0],
            yaw: 17f32.to_radians(),
            pitch: -0.123_456,
            fov_deg: 90.0,
            fly: true,
            width: 1600,
            height: 900,
            bands: Bands::of(&Settings::default()),
            readings: Readings::default(),
        }
    }

    /// The pose survives the file exactly, so a replay looks the same way.
    #[test]
    fn a_sidecar_round_trips_exactly() {
        let s = shot();
        let text = toml::to_string(&s).unwrap();
        let back: Shot = toml::from_str(&text).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.yaw.to_bits(), s.yaw.to_bits());
    }

    #[test]
    fn a_sidecar_without_readings_loads() {
        let mut text = toml::to_string(&shot()).unwrap();
        let cut = text.find("[readings]").unwrap();
        text.truncate(cut);
        let back: Shot = toml::from_str(&text).unwrap();
        assert_eq!(back.readings, Readings::default());
    }

    #[test]
    fn replay_settings_take_the_shot_values() {
        let mut s = shot();
        s.seed = 7;
        s.bands.l3 = 33.0;
        let applied = s.apply_to(&Settings::default());
        assert_eq!(applied.seed, 7);
        assert!((applied.band_l3_m - 33.0).abs() < 1e-9);
        assert_eq!(applied.width, 1600);
    }

    /// The viewpoint lists in `docs/playtests/viewpoints/` are sidecars that load.
    #[test]
    fn viewpoint_lists_load() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/playtests/viewpoints");
        let mut n = 0;
        for dir in std::fs::read_dir(&root).unwrap() {
            for file in std::fs::read_dir(dir.unwrap().path()).unwrap() {
                let path = file.unwrap().path();
                if path.extension().is_some_and(|e| e == "toml") {
                    Shot::load(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                    n += 1;
                }
            }
        }
        assert!(n > 0, "no viewpoints in {}", root.display());
    }

    #[test]
    fn autoshot_modes() {
        assert_eq!(AutoShot::parse("120"), Some(AutoShot::After(120)));
        assert_eq!(
            AutoShot::parse("settle"),
            Some(AutoShot::Settled {
                limit: SETTLE_LIMIT
            })
        );
        assert_eq!(
            AutoShot::parse("settle:500"),
            Some(AutoShot::Settled { limit: 500 })
        );
        assert_eq!(AutoShot::parse("settled"), None);
        assert_eq!(AutoShot::parse("x"), None);
        let s = AutoShot::Settled { limit: 500 };
        assert!(!s.due(10, QUIET_FRAMES - 1));
        assert!(s.due(10, QUIET_FRAMES));
        assert!(s.due(500, 0));
        assert!(AutoShot::After(3).due(3, 0));
    }
}
