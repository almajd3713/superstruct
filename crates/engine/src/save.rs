//! The save file, `save.toml`: player-made marks that outlive a session. Beacons now; the seed,
//! position and safe ground join in P6.2.2.

use std::path::{Path, PathBuf};

use glam::{I64Vec3, Vec3};
use serde::{Deserialize, Serialize};
use worldgen::coord::WorldPos;

use crate::settings::SettingsError;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BeaconPos {
    pub sector: [i64; 3],
    pub local: [f32; 3],
}

impl BeaconPos {
    pub fn of(p: &WorldPos) -> Self {
        let (s, l) = (p.sector(), p.local());
        Self {
            sector: [s.x, s.y, s.z],
            local: [l.x, l.y, l.z],
        }
    }

    /// `None` for a position outside the world's domain.
    pub fn pos(&self) -> Option<WorldPos> {
        let [x, y, z] = self.sector;
        WorldPos::new(I64Vec3::new(x, y, z), Vec3::from(self.local)).ok()
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Save {
    pub beacons: Vec<BeaconPos>,
    /// Pinned far landmarks (TOOL-02): the L1 cell of each.
    pub marks: Vec<[i64; 3]>,
    /// L1 cells arrived in, in order (TOOL-04).
    pub places: Vec<[i64; 3]>,
    /// Marks drawn on walls (TOOL-08).
    pub chalk: Vec<BeaconPos>,
}

impl Save {
    /// An absent file is an empty save; a damaged one is an error.
    pub fn load(path: &Path) -> Result<Self, SettingsError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(toml::from_str(&text)?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// `save.toml` next to the working directory unless `SUPERSTRUCT_SAVE` points elsewhere.
pub fn save_path() -> PathBuf {
    std::env::var_os("SUPERSTRUCT_SAVE").map_or_else(|| PathBuf::from("save.toml"), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beacons_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("ss-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("save.toml");
        assert_eq!(Save::load(&path).unwrap(), Save::default());
        let at = WorldPos::new(
            I64Vec3::new(-46, 1, 7_000_000),
            Vec3::new(404.5, 529.25, 3.0),
        )
        .unwrap();
        let save = Save {
            beacons: vec![BeaconPos::of(&at)],
            places: vec![[1, 2, 3]],
            ..Save::default()
        };
        save.save(&path).unwrap();
        let back = Save::load(&path).unwrap();
        assert_eq!(back, save);
        assert_eq!(back.beacons[0].pos(), Some(at));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_stale_trail_key_in_an_old_save_is_ignored() {
        let text =
            "places = [[1, 2, 3]]\n\n[[trail]]\nsector = [0, 0, 0]\nlocal = [1.0, 2.0, 3.0]\n";
        let save: Save = toml::from_str(text).unwrap();
        assert_eq!(save.places, vec![[1, 2, 3]]);
    }
}
