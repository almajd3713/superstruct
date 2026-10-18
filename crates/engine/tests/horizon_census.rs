#![allow(clippy::unwrap_used)]
use glam::I64Vec3;
use worldgen::cavity::CavityView;
use worldgen::cell::{CellKey, Level};
use worldgen::key::ColumnSpanKey;
use worldgen::l1::{l1_cell, trunk, trunk_piece};
use worldgen::prim::FeatureKind;
use worldgen::seed::WorldSeed;

#[test]
#[ignore = "measurement"]
fn census() {
    let seed = WorldSeed(0);
    let (mut cells, mut cavity, mut trunk_cells, mut trunk_dropped, mut decks) = (0, 0, 0, 0, 0);
    for z in -20..20 {
        for y in -20..20 {
            for x in -20..20 {
                let key = CellKey::new(Level::L1, I64Vec3::new(x, y, z));
                cells += 1;
                if CavityView::new(seed, key).is_cavity(key) {
                    cavity += 1;
                    continue;
                }
                let t =
                    trunk(seed, ColumnSpanKey::of(key)).filter(|t| trunk_piece(t, key).is_some());
                let out = l1_cell(seed, key);
                let has_trunk = out.primitives.iter().any(|p| p.kind == FeatureKind::TRUNK);
                if t.is_some() {
                    trunk_cells += 1;
                    if !has_trunk {
                        trunk_dropped += 1;
                    }
                }
                if out.primitives.iter().any(|p| p.kind == FeatureKind::DECK) {
                    decks += 1;
                }
            }
        }
    }
    eprintln!(
        "{cells} cells: {cavity} cavity, {trunk_cells} with trunk piece, {trunk_dropped} of those dropped by bridges, {decks} with decks"
    );
}
