//! `web-report`: measures the generated world against the targets of `the-web.md` §14 and
//! counts every registry ID that exists in it. Spec: `docs/spec/web-report.md`.
//!
//! This is the L1 version (PW.2.11). Metrics and census sources that need the L2 tissue or
//! L3 interiors report `null` with the item that provides them, and their IDs show up in
//! `zero_ids` until then.

#![allow(
    clippy::too_many_lines,
    clippy::type_complexity,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::write_with_newline
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::time::Instant;

mod tissue;

use anyhow::{Context, Result, bail};
use glam::{I64Vec3, Vec3};
use rayon::prelude::*;
use worldgen::cell::Level;
use worldgen::character::character;
use worldgen::hash::hash;
use worldgen::l1_artery::{ArteryType, Role, skeleton};
use worldgen::l1_between::between_content;
use worldgen::l1_frame::{beams_of_cell, frame_content};
use worldgen::l1_surface::{surface_content, surface_openings};
use worldgen::l1_void::{voids_of_cell, voids_of_l0};
use worldgen::regime::regime;
use worldgen::rng::Rng;
use worldgen::seed::WorldSeed;
use worldgen::stack::{Stratum, stack};

const L1_M: i64 = 4096;
const L0_M: i64 = 32_768;
/// Salt of the sampling RNG (`web-report.md` §6). Used by this tool only.
const WEB_REPORT: u64 = 0x0F10;

const HELP: &str = "web-report: metrics and census of the generated world

  --seed N        world seed (default 42)
  --origin x,y,z  min corner of the region in metres (default 0,0,0)
  --size N        edge of the region in metres, at least 4096 (default 32768)
  --samples N     points for the sampled metrics (default 20000)
  --defects N     count the defects of N sampled L2 cells with their L3 (PW.4.8) and exit
  --tissue N      MET-01, 08 and 10 over N sampled L2 cells and exit
  --threads N     threads for --defects and --tissue (default: all)
  --census        count the instances of every registry ID
  --ids A-,B-     restrict the census output and --strict to these prefixes
  --out FILE      write the JSON report there
  --md FILE       write a Markdown summary there
  --registry FILE the idea registry (default docs/proposals/registry.md)
  --strict        exit 1 if a metric fails or a selected ID has no instance";

struct Args {
    seed: u64,
    origin: I64Vec3,
    size: i64,
    samples: u32,
    census: bool,
    defects: usize,
    tissue: usize,
    threads: usize,
    ids: Vec<String>,
    out: Option<String>,
    md: Option<String>,
    registry: String,
    strict: bool,
}

fn parse() -> Result<Args> {
    let mut a = Args {
        seed: 42,
        origin: I64Vec3::ZERO,
        size: L0_M,
        samples: 20_000,
        census: false,
        defects: 0,
        tissue: 0,
        threads: std::thread::available_parallelism().map_or(1, usize::from),
        ids: Vec::new(),
        out: None,
        md: None,
        registry: "docs/proposals/registry.md".into(),
        strict: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().with_context(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--seed" => a.seed = value("--seed")?.parse()?,
            "--origin" => {
                let v: Vec<i64> = value("--origin")?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
                let [x, y, z] = v[..] else {
                    bail!("--origin needs x,y,z");
                };
                a.origin = I64Vec3::new(x, y, z);
            }
            "--size" => a.size = value("--size")?.parse()?,
            "--samples" => a.samples = value("--samples")?.parse()?,
            "--census" => a.census = true,
            "--defects" => a.defects = value("--defects")?.parse()?,
            "--tissue" => a.tissue = value("--tissue")?.parse()?,
            "--threads" => a.threads = value("--threads")?.parse()?,
            "--ids" => a.ids = value("--ids")?.split(',').map(String::from).collect(),
            "--out" => a.out = Some(value("--out")?),
            "--md" => a.md = Some(value("--md")?),
            "--registry" => a.registry = value("--registry")?,
            "--strict" => a.strict = true,
            "--help" | "-h" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            other => bail!("unknown argument {other}\n{HELP}"),
        }
    }
    if a.size < L1_M {
        bail!("--size must be at least one L1 cell ({L1_M} m)");
    }
    Ok(a)
}

/// One instance: a registry ID, a key that is the same for every cell the feature touches,
/// and the L1 cell it was found in.
struct Found {
    tag: String,
    key: u64,
    cell: I64Vec3,
}

fn key_of<T: Hash>(v: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

/// Instances at the level of an L0 cell: regime, stack, voids, strata characters.
fn l0_instances(seed: WorldSeed, l0: I64Vec3) -> Vec<Found> {
    let mut out = Vec::new();
    let cell = l0 * 8;
    let mut add = |tag: &str, key: u64| {
        out.push(Found {
            tag: tag.to_string(),
            key,
            cell,
        });
    };
    let reg = regime(seed, l0);
    let k0 = key_of(&(l0.x, l0.y, l0.z));
    for id in ["REG-01", "REG-02", "REG-03", "REG-04"] {
        add(id, k0);
    }
    add(reg.web.orientation.tag(), k0);
    if let Some(t) = reg.web.stack.tag() {
        add(t, k0);
    }
    let st = stack(seed, l0);
    for seg in &st.segments {
        let key = key_of(&(l0.x, l0.y, l0.z, seg.start));
        match seg.stratum {
            Stratum::Crust(k) => {
                add("STR-01", key);
                // SC-01..09 are all in the record, so a drawn character counts for each.
                let c = character(seed, l0, k);
                let n = c.dominant().count();
                for (i, id) in [
                    "SC-01", "SC-02", "SC-03", "SC-04", "SC-05", "SC-06", "SC-07", "SC-08", "SC-09",
                ]
                .into_iter()
                .enumerate()
                {
                    // SC-05 counts only where the scale is not ×1.
                    if i == 4 && c.scale == 2 {
                        continue;
                    }
                    let _ = n;
                    add(id, key);
                }
            }
            Stratum::Between(_) => {
                if let Some(kind) = seg.kind {
                    add(kind.tag(), key);
                }
                if seg.flooded {
                    add("BTW-05", key);
                }
            }
        }
    }
    for v in voids_of_l0(seed, l0) {
        for t in v.tags() {
            add(t, v.key);
        }
    }
    out
}

/// Instances inside one L1 cell: beams, Between content, surfaces, voids, the skeleton.
fn l1_instances(seed: WorldSeed, l1: I64Vec3) -> Vec<Found> {
    let mut out = Vec::new();
    let mut add = |tag: &str, key: u64| {
        out.push(Found {
            tag: tag.to_string(),
            key,
            cell: l1,
        });
    };
    for b in beams_of_cell(seed, l1) {
        let key = key_of(&(b.dir, b.p, b.q, b.span));
        for t in b.tags() {
            add(t, key);
        }
    }
    let pieces = frame_content(seed, l1)
        .into_iter()
        .chain(between_content(seed, l1))
        .chain(surface_content(seed, l1));
    for p in pieces {
        let bb = p.prim.aabb();
        add(
            p.tag,
            key_of(&(
                p.tag, bb.min.x, bb.min.y, bb.min.z, bb.max.x, bb.max.y, bb.max.z,
            )),
        );
    }
    for o in surface_openings(seed, l1) {
        add("HOR-02", key_of(&(o.run_key, o.s0, o.t0, o.s1, o.t1)));
    }
    for v in voids_of_cell(seed, l1) {
        let bb = v.piece.prim.aabb();
        add(
            v.piece.tag,
            key_of(&(v.piece.tag, bb.min.x, bb.min.y, bb.min.z)),
        );
    }
    if let Some(s) = skeleton(seed, l1) {
        add("ART-03", key_of(&(l1.x, l1.y, l1.z)));
        add("ART-13", key_of(&(l1.x, l1.y, l1.z)));
        add("FILL-01", key_of(&(l1.x, l1.y, l1.z)));
        for (i, a) in s.arteries.iter().enumerate() {
            if matches!(a.role, Role::Port { .. }) {
                add("ART-01", key_of(&(l1.x, l1.y, l1.z, i)));
            }
            let at = ArteryType::ALL
                .iter()
                .position(|&t| t == a.kind)
                .unwrap_or(0);
            add(
                &format!("ART-{:02}", 4 + at),
                key_of(&(l1.x, l1.y, l1.z, i)),
            );
        }
    }
    out
}

/// MET-02: the share of 1 m probes in Betweens that lie inside some L1 piece.
fn between_built_fraction(
    seed: WorldSeed,
    l1s: &[I64Vec3],
    samples: u32,
    origin: I64Vec3,
) -> (f64, u32) {
    let mut rng = Rng::new(hash(seed.0, Level::L0, origin / L0_M, WEB_REPORT));
    let per_cell = (samples as usize / l1s.len().max(1)).max(8);
    let draws: Vec<(I64Vec3, Vec<Vec3>)> = l1s
        .iter()
        .map(|&l1| {
            let pts = (0..per_cell)
                .map(|_| {
                    Vec3::new(
                        rng.next_f32() * 4096.0,
                        rng.next_f32() * 4096.0,
                        rng.next_f32() * 4096.0,
                    )
                })
                .collect();
            (l1, pts)
        })
        .collect();
    let (built, total) = draws
        .par_iter()
        .map(|(l1, pts)| {
            let l0 = l1.div_euclid(I64Vec3::splat(8));
            let st = stack(seed, l0);
            let pieces: Vec<_> = frame_content(seed, *l1)
                .into_iter()
                .chain(between_content(seed, *l1))
                .chain(surface_content(seed, *l1))
                .collect();
            let (mut built, mut total) = (0u32, 0u32);
            for p in pts {
                let w = *l1 * L1_M + I64Vec3::new(p.x as i64, p.y as i64, p.z as i64);
                let local = w.rem_euclid(I64Vec3::splat(L0_M));
                if matches!(st.stratum_at(local, None), Stratum::Between(_)) {
                    total += 1;
                    built += u32::from(pieces.iter().any(|q| q.prim.shape.contains(*p)));
                }
            }
            (built, total)
        })
        .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
    (f64::from(built) / f64::from(total.max(1)), total)
}

/// The IDs of the registry above its "Deferred" section, in file order.
fn registry_ids(path: &str) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
    let mut ids = Vec::new();
    for line in text.lines() {
        if line.starts_with("## Deferred") {
            break;
        }
        let Some(cell) = line.strip_prefix("| ").and_then(|l| l.split(" |").next()) else {
            continue;
        };
        let ok = cell.split_once('-').is_some_and(|(p, n)| {
            !p.is_empty()
                && p.chars().all(|c| c.is_ascii_uppercase())
                && !n.is_empty()
                && n.chars().all(|c| c.is_ascii_digit())
        });
        if ok {
            ids.push(cell.to_string());
        }
    }
    Ok(ids)
}

/// IDs that no generator output carries but a named test exercises (`web-report.md` §4).
const TESTED: &[(&str, &str, &str)] = &[(
    "ART-02",
    "bfs_finishes_a_walk_with_no_budget_left",
    "crates/gen/src/l1_artery.rs",
)];

fn wanted(id: &str, prefixes: &[String]) -> bool {
    prefixes.is_empty() || prefixes.iter().any(|p| id.starts_with(p.as_str()))
}

struct Metric {
    id: &'static str,
    what: &'static str,
    target: (f64, f64),
    value: Option<f64>,
    n: u32,
    note: &'static str,
}

impl Metric {
    fn pass(&self) -> bool {
        self.value
            .is_some_and(|v| v >= self.target.0 && v <= self.target.1)
    }
}

fn main() -> Result<()> {
    let a = parse()?;
    let seed = WorldSeed(a.seed);
    let t0 = Instant::now();
    if a.defects > 0 {
        return defects(&a, seed, t0);
    }
    if a.tissue > 0 {
        return tissue_run(&a, seed);
    }
    let lo = a.origin.div_euclid(I64Vec3::splat(L1_M));
    let hi = (a.origin + I64Vec3::splat(a.size - 1)).div_euclid(I64Vec3::splat(L1_M));
    let mut l1s = Vec::new();
    for z in lo.z..=hi.z {
        for y in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                l1s.push(I64Vec3::new(x, y, z));
            }
        }
    }
    let l0lo = lo.div_euclid(I64Vec3::splat(8));
    let l0hi = hi.div_euclid(I64Vec3::splat(8));
    let mut l0s = Vec::new();
    for z in l0lo.z..=l0hi.z {
        for y in l0lo.y..=l0hi.y {
            for x in l0lo.x..=l0hi.x {
                l0s.push(I64Vec3::new(x, y, z));
            }
        }
    }
    eprintln!("region: {} L1 cells in {} L0 cells", l1s.len(), l0s.len());

    let (frac, in_between) = between_built_fraction(seed, &l1s, a.samples, a.origin);
    let later = |id, what, target, note| Metric {
        id,
        what,
        target,
        value: None,
        n: 0,
        note,
    };
    let metrics = [
        later(
            "MET-01",
            "Built fraction in crusts",
            (0.70, 0.90),
            "needs the L2 tissue (PW.3.2)",
        ),
        Metric {
            id: "MET-02",
            what: "Built fraction in Betweens (L1 pieces only)",
            target: (0.02, 0.10),
            value: Some(frac),
            n: in_between,
            note: "",
        },
        later(
            "MET-03",
            "Median sightline in crusts (m)",
            (30.0, 150.0),
            "needs L2 and the SDF at L1+L2 (PW.3.2)",
        ),
        later(
            "MET-04",
            "Walking distance to a 2 km sightline (m)",
            (1500.0, 2500.0),
            "needs the L2 nav graph (PW.3.1)",
        ),
        later(
            "MET-05",
            "Walking distance to breathing space (m)",
            (0.0, 150.0),
            "needs L2 payoffs (PW.3.19)",
        ),
        later(
            "MET-06",
            "Crust skin covered by L2",
            (0.90, 1.0),
            "needs L2 accretion (PW.3.2)",
        ),
        later(
            "MET-07",
            "Vertical share of walked distance",
            (0.33, 1.0),
            "needs bots on the L2 nav graph (PW.3)",
        ),
        later(
            "MET-08",
            "Distinct path types per km",
            (8.0, f64::MAX),
            "needs L2 paths (PW.3.9)",
        ),
        later(
            "MET-09",
            "Dead ends without a payoff",
            (0.0, 0.0),
            "needs L2 payoffs (PW.3.18)",
        ),
        later(
            "MET-10",
            "Nav nodes with three or more links",
            (0.40, 1.0),
            "needs the L2 nav graph (PW.3.1)",
        ),
    ];

    let mut census: BTreeMap<String, (BTreeSet<u64>, BTreeSet<I64Vec3Key>, (u64, I64Vec3))> =
        BTreeMap::new();
    let mut zero_ids = Vec::new();
    let mut tested = Vec::new();
    if a.census {
        let mut found: Vec<Found> = l0s
            .par_iter()
            .flat_map(|&l0| l0_instances(seed, l0))
            .collect();
        let per_l1: Vec<Vec<Found>> = l1s.par_iter().map(|&l1| l1_instances(seed, l1)).collect();
        found.extend(per_l1.into_iter().flatten());
        for f in found {
            let e = census
                .entry(f.tag)
                .or_insert_with(|| (BTreeSet::new(), BTreeSet::new(), (u64::MAX, f.cell)));
            e.0.insert(f.key);
            e.1.insert(I64Vec3Key(f.cell.to_array()));
            if f.key < e.2.0 {
                e.2 = (f.key, f.cell);
            }
        }
        for (id, test, file) in TESTED {
            let src = std::fs::read_to_string(file).unwrap_or_default();
            if src.contains(&format!("fn {test}")) {
                tested.push((*id, *test));
            }
        }
        for id in registry_ids(&a.registry)? {
            if wanted(&id, &a.ids)
                && !census.contains_key(&id)
                && !tested.iter().any(|(t, _)| *t == id)
            {
                zero_ids.push(id);
            }
        }
    }

    let mut json = String::new();
    let _ = write!(
        json,
        "{{\n  \"web_report\": 1, \"gen_version\": {}, \"seed\": {},\n  \"regions\": [{{ \"origin\": [{}, {}, {}], \"size\": {} }}],\n  \"metrics\": {{\n",
        worldgen::GEN_VERSION,
        a.seed,
        a.origin.x,
        a.origin.y,
        a.origin.z,
        a.size
    );
    for (i, m) in metrics.iter().enumerate() {
        let value = m.value.map_or("null".to_string(), |v| format!("{v:.4}"));
        let hi = if m.target.1 == f64::MAX {
            "null".to_string()
        } else {
            m.target.1.to_string()
        };
        let _ = write!(
            json,
            "    \"{}\": {{ \"value\": {value}, \"target\": [{}, {hi}], \"pass\": {}, \"n\": {}, \"reason\": \"{}\" }}{}\n",
            m.id,
            m.target.0,
            m.pass(),
            m.n,
            m.note,
            if i + 1 < metrics.len() { "," } else { "" }
        );
    }
    json.push_str("  },\n  \"census\": {\n");
    let rows: Vec<_> = census.iter().filter(|(id, _)| wanted(id, &a.ids)).collect();
    for (i, (id, (keys, cells, (_, first)))) in rows.iter().enumerate() {
        let _ = write!(
            json,
            "    \"{id}\": {{ \"count\": {}, \"cells\": {}, \"first_cell\": [{}, {}, {}] }}{}\n",
            keys.len(),
            cells.len(),
            first.x,
            first.y,
            first.z,
            if i + 1 < rows.len() { "," } else { "" }
        );
    }
    let list = |v: &[String]| {
        v.iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let _ = write!(
        json,
        "  }},\n  \"tested\": [{}],\n  \"zero_ids\": [{}],\n  \"timing\": {{ \"total_s\": {:.1} }}\n}}\n",
        tested
            .iter()
            .map(|(i, t)| format!("\"{i}: {t}\""))
            .collect::<Vec<_>>()
            .join(", "),
        list(&zero_ids),
        t0.elapsed().as_secs_f64()
    );

    let mut md = String::from("| ID | Metric | Value | Target | Pass |\n|---|---|---|---|---|\n");
    for m in &metrics {
        let v = m
            .value
            .map_or_else(|| format!("null ({})", m.note), |v| format!("{v:.3}"));
        let _ = writeln!(
            md,
            "| {} | {} | {v} | {}–{} | {} |",
            m.id,
            m.what,
            m.target.0,
            if m.target.1 == f64::MAX {
                "∞".into()
            } else {
                m.target.1.to_string()
            },
            m.pass()
        );
    }
    if a.census {
        md.push_str("\n| ID | Instances | Cells | First L1 cell |\n|---|---|---|---|\n");
        for (id, (keys, cells, (_, f))) in &rows {
            let _ = writeln!(
                md,
                "| {id} | {} | {} | {},{},{} |",
                keys.len(),
                cells.len(),
                f.x,
                f.y,
                f.z
            );
        }
        let _ = writeln!(md, "\nIDs without an instance: {}", zero_ids.len());
    }

    match &a.out {
        Some(p) => {
            if let Some(dir) = std::path::Path::new(p).parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(p, &json)?;
        }
        None => print!("{json}"),
    }
    if let Some(p) = &a.md {
        if let Some(dir) = std::path::Path::new(p).parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(p, &md)?;
    } else if a.out.is_some() {
        println!("{md}");
    }
    let failed = metrics.iter().any(|m| !m.pass());
    if a.strict && (failed || !zero_ids.is_empty()) {
        std::process::exit(1);
    }
    Ok(())
}

/// `I64Vec3` is not `Ord`; a plain array is.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct I64Vec3Key([i64; 3]);

/// `--tissue`: the L2 metrics over sampled cells of the region.
fn tissue_run(a: &Args, seed: WorldSeed) -> Result<()> {
    let cells = sample_cells(a, a.tissue);
    tissue::run(seed, &cells, a.threads)
}

/// `n` distinct L2 cells drawn from the region by the sampling RNG.
#[allow(clippy::cast_possible_wrap)]
fn sample_cells(a: &Args, n: usize) -> Vec<I64Vec3> {
    let lo = a.origin.div_euclid(I64Vec3::splat(512));
    let span = (a.size / 512).max(1);
    let mut state = a.seed ^ 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    let mut cells = std::collections::BTreeSet::new();
    while cells.len() < n {
        let d = I64Vec3::new(
            (next() % span as u64) as i64,
            (next() % span as u64) as i64,
            (next() % span as u64) as i64,
        );
        cells.insert((lo + d).to_array());
    }
    cells.into_iter().map(I64Vec3::from_array).collect()
}

/// `--defects`: the defect census over sampled L2 cells of the region.
#[allow(clippy::unnecessary_wraps, clippy::cast_possible_wrap)]
fn defects(a: &Args, seed: WorldSeed, t0: Instant) -> Result<()> {
    let lo = a.origin.div_euclid(I64Vec3::splat(512));
    let span = (a.size / 512).max(1);
    let mut state = a.seed ^ 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    let mut cells = std::collections::BTreeSet::new();
    while cells.len() < a.defects {
        let d = I64Vec3::new(
            (next() % span as u64) as i64,
            (next() % span as u64) as i64,
            (next() % span as u64) as i64,
        );
        cells.insert((lo + d).to_array());
    }
    let cells: Vec<I64Vec3> = cells.into_iter().map(I64Vec3::from_array).collect();
    let r = worldgen::defects::scan_cells(seed, &cells, a.threads);
    println!(
        "defect census: seed {}, {} cells, {:.0} s",
        a.seed,
        r.cells,
        t0.elapsed().as_secs_f64()
    );
    for (k, v) in &r.counts {
        println!("  {k:<14} {v}");
    }
    println!("measured:");
    for (k, v) in &r.bases {
        println!("  {k:<14} {v}");
    }
    for e in &r.examples {
        println!("  eg {e}");
    }
    Ok(())
}
