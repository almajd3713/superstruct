//! `bots`: headless walkers on the generated world (P6.1.1; `docs/spec/web-report.md` §3).
//!
//! Loads a block of L1 cells (their L2 cells and the tissue's payoffs and walks), builds one nav
//! graph of the walkable plates across all of it, links plates across L2 faces, and measures what
//! needs a walker: the walking distance to a long view (MET-04) and to near breathing space
//! (MET-05), the vertical share and the path types per km of walked distance (MET-07, MET-08)
//! and the share of nodes with three or more links (MET-10). The long views are found by rays
//! through an occupancy grid of 16 m cells (the tissue's envelopes and the L1 pieces).

#![allow(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::too_many_arguments,
    clippy::similar_names
)]

mod graph;

use std::time::Instant;

use anyhow::{Context, Result, bail};
use glam::I64Vec3;

const HELP: &str = "bots: walkers on the generated world

  --seed N        world seed (default 42)
  --l1 x,y,z      the low corner of the block, in L1 cells (default 0,0,0)
  --blocks N      the block is N x N x N L1 cells (default 2)
  --starts N      start points for MET-04 and MET-05 (default 2000)
  --bots N        walkers for MET-07 and MET-08 (default 40)
  --bot-km N      kilometres each walker walks (default 2)
  --threads N     threads (default: all)";

pub struct Opts {
    pub seed: u64,
    pub low: I64Vec3,
    pub blocks: i64,
    pub starts: usize,
    pub bots: usize,
    pub bot_km: f64,
    pub threads: usize,
}

fn parse() -> Result<Opts> {
    let mut o = Opts {
        seed: 42,
        low: I64Vec3::ZERO,
        blocks: 2,
        starts: 2000,
        bots: 40,
        bot_km: 2.0,
        threads: std::thread::available_parallelism().map_or(1, usize::from),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().with_context(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--seed" => o.seed = value("--seed")?.parse()?,
            "--l1" => {
                let v: Vec<i64> = value("--l1")?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
                let [x, y, z] = v[..] else {
                    bail!("--l1 needs x,y,z");
                };
                o.low = I64Vec3::new(x, y, z);
            }
            "--blocks" => o.blocks = value("--blocks")?.parse()?,
            "--starts" => o.starts = value("--starts")?.parse()?,
            "--bots" => o.bots = value("--bots")?.parse()?,
            "--bot-km" => o.bot_km = value("--bot-km")?.parse()?,
            "--threads" => o.threads = value("--threads")?.parse()?,
            "--help" | "-h" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            other => bail!("unknown argument {other}\n{HELP}"),
        }
    }
    Ok(o)
}

fn main() -> Result<()> {
    let o = parse()?;
    let t0 = Instant::now();
    let g = graph::load(&o)?;
    eprintln!(
        "loaded {} cells, {} plates, {} links in {:.0} s",
        g.cells,
        g.nodes(),
        g.links(),
        t0.elapsed().as_secs_f64()
    );
    let r = graph::measure(&g, &o);
    println!(
        "bots: seed {}, block {} L1 from {}, {} cells, {} plates, {:.0} s",
        o.seed,
        o.blocks,
        o.low,
        g.cells,
        g.nodes(),
        t0.elapsed().as_secs_f64()
    );
    println!("{r}");
    Ok(())
}
