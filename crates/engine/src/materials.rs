//! The material table (`worldgen::prim::MaterialId` → surface look). The palette stays narrow:
//! a handful of greys, one warm tone for sanctuaries and one emissive white, in the spirit of
//! ink work. Uploaded as part of the globals and read by `shade.wgsl`.

use worldgen::prim::MaterialId;

/// Slots in the GPU table; ids at or above `MaterialId`'s highest are the fallback grey.
pub const SLOTS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material {
    /// Linear base tone.
    pub albedo: [f32; 3],
    /// 0 is glossy, 1 is matte: how much sheen the sun draws on the surface.
    pub roughness: f32,
    /// 0 to 1: how much of the base tone shows regardless of light.
    pub emissive: f32,
}

const fn m(albedo: [f32; 3], roughness: f32, emissive: f32) -> Material {
    Material {
        albedo,
        roughness,
        emissive,
    }
}

const FALLBACK: Material = m([0.4, 0.4, 0.4], 0.8, 0.0);

/// Indexed by `MaterialId`.
pub const TABLE: [Material; SLOTS] = {
    let mut t = [FALLBACK; SLOTS];
    t[MaterialId::CONCRETE.0 as usize] = m([0.36, 0.37, 0.39], 0.9, 0.0);
    t[MaterialId::METAL.0 as usize] = m([0.2, 0.21, 0.24], 0.45, 0.0);
    t[MaterialId::GRATING.0 as usize] = m([0.14, 0.15, 0.17], 0.6, 0.0);
    t[MaterialId::EMISSIVE.0 as usize] = m([0.9, 0.88, 0.8], 1.0, 0.85);
    t[MaterialId::PIPE.0 as usize] = m([0.27, 0.29, 0.29], 0.55, 0.0);
    t[MaterialId::WARM.0 as usize] = m([0.62, 0.46, 0.3], 0.8, 0.25);
    t[MaterialId::BEACON.0 as usize] = m([1.0, 0.8, 0.45], 1.0, 1.0);
    t
};

/// The table as the shader reads it: `SLOTS` vec4s of albedo and emissive, then `SLOTS` vec4s
/// holding roughness in x.
pub fn gpu_table() -> [[f32; 4]; 2 * SLOTS] {
    let mut out = [[0.0; 4]; 2 * SLOTS];
    for (i, mat) in TABLE.iter().enumerate() {
        out[i] = [mat.albedo[0], mat.albedo[1], mat.albedo[2], mat.emissive];
        out[SLOTS + i] = [mat.roughness, 0.0, 0.0, 0.0];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_material_id_has_an_entry_and_values_are_in_range() {
        for id in [
            MaterialId::CONCRETE,
            MaterialId::METAL,
            MaterialId::GRATING,
            MaterialId::EMISSIVE,
            MaterialId::PIPE,
            MaterialId::WARM,
            MaterialId::BEACON,
        ] {
            let mat = TABLE[usize::from(id.0)];
            assert_ne!(mat, FALLBACK, "id {} has no entry", id.0);
        }
        for mat in TABLE {
            assert!(mat.albedo.iter().all(|c| (0.0..=1.0).contains(c)));
            assert!((0.0..=1.0).contains(&mat.roughness));
            assert!((0.0..=1.0).contains(&mat.emissive));
        }
    }

    #[test]
    fn the_palette_is_narrow() {
        // Non-emissive materials differ in tone by little hue: channels stay within 0.1 of each
        // other, except the one warm tone.
        for (i, mat) in TABLE.iter().enumerate().take(6) {
            if i == usize::from(MaterialId::WARM.0) {
                continue;
            }
            let [r, g, b] = mat.albedo;
            assert!(r.max(g).max(b) - r.min(g).min(b) <= 0.1, "id {i}");
        }
    }

    #[test]
    fn only_the_emissive_materials_glow() {
        for (i, mat) in TABLE.iter().enumerate() {
            let glows = mat.emissive > 0.0;
            let expect = [MaterialId::EMISSIVE, MaterialId::WARM, MaterialId::BEACON]
                .iter()
                .any(|m| usize::from(m.0) == i);
            assert_eq!(glows, expect, "id {i}");
        }
    }

    #[test]
    fn the_gpu_table_matches() {
        let t = gpu_table();
        assert!((t[3][3] - TABLE[3].emissive).abs() < 1e-6);
        assert!((t[SLOTS + 1][0] - TABLE[1].roughness).abs() < 1e-6);
    }
}
