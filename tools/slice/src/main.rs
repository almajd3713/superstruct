//! Cross-sections and connectivity reports from the generator.
//!
//! ```text
//! slice --seed 42 --level 2 --axis y --at 1000 --center 0,0 --extent 4096 --res 1024 --out out/s.png --grid --ports --walkable
//! slice --report --seed 42 --count 10000 --out out/report.json
//! ```

// Pixel and quanta conversions are deliberate, and bounded by the image size.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

mod render;
mod stack_view;

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use worldgen::cell::Axis;
use worldgen::seed::WorldSeed;
use worldgen::validate::check_random;

const HELP: &str = "\
slice: cross-sections and connectivity reports from the generator

USAGE:
    slice [OPTIONS]
    slice --report [OPTIONS]

SECTION OPTIONS:
    --seed <TEXT|N>      World seed: a number, or text that is hashed. Default: 42
    --level <1|2>        Finest level generated and drawn. 1 = L1 megaforms only,
                         2 = L1 plus L2 districts (slow beyond ~8 km). Default: 1
    --axis <x|y|z>       Axis the plane is perpendicular to. Default: y
                         y: plane coordinates are (x right, z down); x: (z right, y up);
                         z: (x right, y up)
    --at <METRES>        Position of the plane along the axis (decimals allowed). Default: 0
    --center <U,V>       Image centre in plane coordinates, whole metres. Default: 0,0
    --extent <METRES>    Width and height of the image in metres. Default: 4096
    --res <PIXELS>       Image width and height in pixels. Default: 1024
    --out <PATH>         Output PNG (or JSON with --report). Default: out/slice.png
    --grid               Overlay cell boundaries (L1 white, L2 grey)
    --ports              Overlay ports that cut the plane (magenta)
    --walkable           Overlay walkable surfaces that cut the plane (green)
    --stack              Draw the regime stacks instead: crusts light, Betweens dark by kind,
                         flooded ones blue, L0 cell edges red (use --extent 32768 or more)
    --fill               Draw the fill field, one grey per L2 cell
    --between            Draw Between content over the stacks, one colour per STR-ID

REPORT OPTIONS (with --report):
    --report             Run the connectivity check over random cells instead of drawing
    --count <N>          Cells to check. Default: 10240
    --extent <METRES>    Side of the sampled region in metres. Default: 4096 (rounded to L1 cells)
    --sample-seed <N>    Seed for choosing cells. Default: 7
    --threads <N>        Worker threads. Default: all cores
    --out <PATH>         JSON report. Default: out/report.json

    -h, --help           Print this help
";

#[allow(clippy::struct_excessive_bools)] // one bool per command-line switch
struct Args {
    seed: WorldSeed,
    level: u8,
    axis: Axis,
    at: f64,
    center: (i64, i64),
    extent: f64,
    res: u32,
    out: Option<PathBuf>,
    grid: bool,
    ports: bool,
    walkable: bool,
    stack: bool,
    between: bool,
    fill: bool,
    report: bool,
    count: u32,
    sample_seed: u64,
    threads: usize,
}

fn parse() -> Result<Option<Args>> {
    let mut a = Args {
        seed: WorldSeed(42),
        level: 1,
        axis: Axis::Y,
        at: 0.0,
        center: (0, 0),
        extent: 4096.0,
        res: 1024,
        out: None,
        grid: false,
        ports: false,
        walkable: false,
        stack: false,
        between: false,
        fill: false,
        report: false,
        count: 10_240,
        sample_seed: 7,
        threads: std::thread::available_parallelism().map_or(2, usize::from),
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = |name: &str| it.next().with_context(|| format!("{name} needs a value"));
        match flag.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(None);
            }
            "--seed" => a.seed = WorldSeed::parse(&value("--seed")?)?,
            "--level" => a.level = value("--level")?.parse().context("--level")?,
            "--axis" => {
                a.axis = match value("--axis")?.as_str() {
                    "x" => Axis::X,
                    "y" => Axis::Y,
                    "z" => Axis::Z,
                    other => bail!("--axis must be x, y or z, not {other:?}"),
                }
            }
            "--at" => a.at = value("--at")?.parse().context("--at")?,
            "--center" => {
                let text = value("--center")?;
                let (u, v) = text.split_once(',').context("--center wants U,V")?;
                a.center = (
                    u.trim().parse().context("--center U")?,
                    v.trim().parse().context("--center V")?,
                );
            }
            "--extent" => a.extent = value("--extent")?.parse().context("--extent")?,
            "--res" => a.res = value("--res")?.parse().context("--res")?,
            "--out" => a.out = Some(PathBuf::from(value("--out")?)),
            "--grid" => a.grid = true,
            "--ports" => a.ports = true,
            "--walkable" => a.walkable = true,
            "--stack" => a.stack = true,
            "--between" => a.between = true,
            "--fill" => a.fill = true,
            "--report" => a.report = true,
            "--count" => a.count = value("--count")?.parse().context("--count")?,
            "--sample-seed" => {
                a.sample_seed = value("--sample-seed")?.parse().context("--sample-seed")?;
            }
            "--threads" => a.threads = value("--threads")?.parse().context("--threads")?,
            other => bail!("unknown option {other:?}; see --help"),
        }
    }
    if !(1..=2).contains(&a.level) {
        bail!("--level must be 1 or 2");
    }
    if a.res == 0 || a.res > 8192 || a.extent <= 0.0 {
        bail!("--res must be 1..=8192 and --extent positive");
    }
    Ok(Some(a))
}

fn main() -> Result<()> {
    let Some(a) = parse()? else { return Ok(()) };
    if a.report {
        report(&a)
    } else {
        let out = a
            .out
            .clone()
            .unwrap_or_else(|| PathBuf::from("out/slice.png"));
        let view = render::View {
            axis: a.axis,
            at_m: a.at,
            center: a.center,
            extent_m: a.extent,
            res: a.res,
        };
        let opts = render::Options {
            level: a.level,
            grid: a.grid,
            ports: a.ports,
            walkable: a.walkable,
        };
        let start = Instant::now();
        let image = if a.fill {
            stack_view::render_fill(a.seed, &view)
        } else if a.between {
            stack_view::render_content(a.seed, &view)
        } else if a.stack {
            stack_view::render(a.seed, &view)
        } else {
            render::render(a.seed, &view, &opts)
        };
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir)?;
        }
        image
            .save(&out)
            .with_context(|| format!("writing {}", out.display()))?;
        println!("wrote {} in {:.2?}", out.display(), start.elapsed());
        Ok(())
    }
}

fn report(a: &Args) -> Result<()> {
    let out = a
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from("out/report.json"));
    let extent = a.extent as i64;
    let s = check_random(a.seed, a.sample_seed, a.count, extent.max(8), a.threads);
    let failures: Vec<String> = s
        .failures
        .iter()
        .map(|(cell, msg)| {
            let c = cell.coords;
            format!(
                "    {{\"cell\": [{}, {}, {}], \"message\": {:?}}}",
                c.x, c.y, c.z, msg
            )
        })
        .collect();
    let json = format!(
        "{{\n  \"seed\": {},\n  \"sample_seed\": {},\n  \"cells\": {},\n  \"ports\": {},\n  \"walkway_ports\": {},\n  \"threads\": {},\n  \"elapsed_ms\": {},\n  \"failure_count\": {},\n  \"failures\": [\n{}\n  ]\n}}\n",
        a.seed.0,
        a.sample_seed,
        s.cells,
        s.ports,
        s.walkway_ports,
        a.threads,
        s.elapsed.as_millis(),
        s.failures.len(),
        failures.join(",\n"),
    );
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&out, json)?;
    println!(
        "{} cells, {} ports, {} failures in {:.2?}; wrote {}",
        s.cells,
        s.ports,
        s.failures.len(),
        s.elapsed,
        out.display()
    );
    if s.failures.is_empty() {
        Ok(())
    } else {
        bail!("{} connectivity failures", s.failures.len())
    }
}
