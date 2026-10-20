//! ORI-01..11 (PW.3.20): the ideas of free orientation are in the generated world. Most were
//! built with the item that owns their shape (forms, variants, paths, frames, strata); this
//! test finds each of them by census, so none is left out. ORI-10 (radial districts) waits for
//! the gravity wells (PW.3.20b).

use glam::I64Vec3;
use worldgen::cell::{CellKey, Level};
use worldgen::regime::regime;
use worldgen::seed::WorldSeed;
use worldgen::stack::stack;
use worldgen::tissue::{TissueCache, tissue_cell};

fn seed() -> WorldSeed {
    WorldSeed(0x5eed)
}

/// ORI-01 grain, ORI-03 tilted strata, ORI-04 oblique struts, ORI-05 stair beams (L1 side).
#[test]
fn frames_strata_and_beams() {
    use std::collections::HashSet;
    use worldgen::regime_web::StackGeometry;
    // Grain: regions have their own rotation.
    let mut frames = HashSet::new();
    let mut tilted = false;
    for x in -4..4 {
        for z in -4..4 {
            let l0 = I64Vec3::new(x, 0, z);
            let web = regime(seed(), l0).web;
            for k in 0..8 {
                frames.insert(web.frame_l2(seed(), l0 * 64 + I64Vec3::new(k, 3, k * 5 % 8)));
            }
            tilted |= matches!(stack(seed(), l0).geometry, StackGeometry::Tilted { .. });
        }
    }
    assert!(frames.len() > 4, "ORI-01: {} frames", frames.len());
    assert!(tilted, "ORI-03: no tilted stack in 64 L0 cells");
    // Oblique beams and stair beams.
    let (mut oblique, mut stair) = (0, 0);
    for k in 0..400i64 {
        let l1 = I64Vec3::new(k % 9 - 4, k / 9 % 5, k / 45 % 9 - 4);
        for b in worldgen::l1_frame::beams_of_cell(seed(), l1) {
            let tags = b.tags();
            oblique += usize::from(tags.contains(&"FRM-02"));
            stair += usize::from(tags.contains(&"FRM-11"));
        }
    }
    assert!(oblique > 0, "ORI-04: no oblique beam");
    assert!(stair > 0, "ORI-05: no stair beam");
}

/// ORI-02 fallen towers as bridges, ORI-05 funiculars, ORI-06 rotundas, drums and domes,
/// ORI-07 true helices, ORI-08 catenaries, ORI-09 polygonal rings, ORI-11 diagonal routes.
#[test]
fn tissue_ideas() {
    let cache = TissueCache::default();
    let mut forms = [0usize; 23];
    let mut variants = [0usize; 27];
    let mut paths = [0usize; 41];
    for k in 0..1500i64 {
        let l1 = I64Vec3::new(k % 9 - 4, k / 9 % 3, k / 27 % 20 - 10);
        let key = CellKey::new(
            Level::L2,
            l1 * 8 + I64Vec3::new(k % 8, k / 8 % 8, (k * 7 + k / 5) % 8),
        );
        let t = tissue_cell(seed(), &cache, key);
        for s in &t.structures {
            if usize::from(s.form) < 23 {
                forms[usize::from(s.form)] += 1;
            }
            variants[usize::from(s.variant)] += 1;
        }
        for &(_, id) in &t.paths.chosen {
            paths[usize::from(id)] += 1;
        }
    }
    eprintln!("forms {forms:?}\nvariants {variants:?}\npaths {paths:?}");
    assert!(variants[11] > 0, "ORI-02: SV-11 fallen tower");
    assert!(paths[39] > 0, "ORI-05: PV-19 funicular");
    for (form, tag) in [
        (8, "FORM-09 drum"),
        (15, "FORM-16 rotunda"),
        (18, "FORM-19 dome"),
    ] {
        assert!(forms[form] > 0, "ORI-06: {tag}");
    }
    assert!(paths[25] > 0, "ORI-07: PV-05 helix");
    assert!(paths[4] > 0, "ORI-08: PH-04 catenary");
    assert!(forms[19] > 0, "ORI-09: FORM-20 polygonal ring");
    assert!(paths[19] > 0, "ORI-11: PH-19 skew corridor");
}
